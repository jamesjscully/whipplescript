//! Slice-1 file tool layer for the owned brokered harness.
//!
//! Defines the model-facing coding tools (Pi-style: read/write/edit/grep/find/ls)
//! and a [`FileToolExecutor`] that runs each one through the `file store` policy
//! boundary (the same `file_path_policy_error` check the `file.*` effects use).
//! The executor is the concrete [`ToolExecutor`] the kernel's generic brokered
//! loop drives; tool calls are stream events (evidence), never durable effects
//! (DR-0024, spec/owned-harness-loop-contract.md).
//!
//! Slice 1 keeps the search/list tools std-only and deliberately simple
//! (substring grep, glob `find`, plain `ls`); gitignore-awareness and regex are
//! later refinements. `bash` and the budget/lease envelope are later slices.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use whipplescript_kernel::coerce_native::CoerceProvider;
use whipplescript_kernel::harness_loop::{
    BrokeredTurnInput, ChatMessage, HarnessModelClient, HarnessModelError, ModelReply, ToolCall,
    ToolExecutor, ToolOutcome, ToolSpec, ToolStatus,
};
use whipplescript_kernel::harness_model::RealHarnessModelClient;
use whipplescript_kernel::{BrokeredTurnContext, RuntimeKernel};
use whipplescript_store::coordination::{AcquireOutcome, CoordinationStore};
use whipplescript_store::items::WorkItemStore;
use whipplescript_store::{StoreError, StoreResult, StoredEvent};

use crate::coerce_runtime::{resolve_credential_with_source, UreqCoerceTransport};

pub const TOOL_READ: &str = "read";
pub const TOOL_WRITE: &str = "write";
pub const TOOL_EDIT: &str = "edit";
pub const TOOL_GREP: &str = "grep";
pub const TOOL_FIND: &str = "find";
pub const TOOL_LS: &str = "ls";
pub const TOOL_BASH: &str = "bash";
pub const TOOL_LIST_TODOS: &str = "list_todos";
pub const TOOL_ADD_TODO: &str = "add_todo";
pub const TOOL_UPDATE_TODO: &str = "update_todo";

/// Default wall-clock cap for a single `bash` command, in seconds.
const BASH_DEFAULT_TIMEOUT_SECS: u64 = 30;

/// The tracker tools (slice 4): the agent participates in durable shared work
/// state. Offered only when a tracker queue is configured
/// (`WHIPPLESCRIPT_HARNESS_TRACKER`); facades over the builtin work tracker.
pub fn tracker_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: TOOL_LIST_TODOS.into(),
            description: "List work-tracker items (optionally filtered by status).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: TOOL_ADD_TODO.into(),
            description:
                "File a new work-tracker item (a durable to-do the workflow can react to).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string" },
                    "status": { "type": "string", "enum": ["pending"] }
                },
                "required": ["content"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: TOOL_UPDATE_TODO.into(),
            description: "Change a tracker item's status by id.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                },
                "required": ["id", "status"],
                "additionalProperties": false
            }),
        },
    ]
}

/// Default cap on a single tool's returned content. Full output recovery by event
/// reference is a later slice; for now we bound what the model sees.
const DEFAULT_MAX_BYTES: usize = 50_000;
/// Bound on files visited by `find`/`grep` so a huge tree cannot stall a turn.
const MAX_FILES_WALKED: usize = 5_000;

/// The model-facing tool specs (names + JSON schemas) offered to the model.
pub fn file_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: TOOL_READ.into(),
            description: "Read a file's text. Optional 1-based line offset and limit.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "workspace-relative path" },
                    "offset": { "type": "integer", "description": "1-based first line" },
                    "limit": { "type": "integer", "description": "max lines to return" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: TOOL_WRITE.into(),
            description: "Create or overwrite a file with the given content.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: TOOL_EDIT.into(),
            description: "Exact string-replace edits in an existing file. Each oldText must \
                          match a unique region of the current file."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "oldText": { "type": "string" },
                                "newText": { "type": "string" }
                            },
                            "required": ["oldText", "newText"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["path", "edits"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: TOOL_GREP.into(),
            description: "Search file contents for a substring; returns path:line:text.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string", "description": "subdir to search, default root" },
                    "ignoreCase": { "type": "boolean" },
                    "limit": { "type": "integer" }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: TOOL_FIND.into(),
            description: "Find files whose workspace-relative path matches a glob pattern.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "glob, e.g. **/*.rs" },
                    "path": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: TOOL_LS.into(),
            description: "List a directory's entries (directories marked with a trailing /)."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "default workspace root" },
                    "limit": { "type": "integer" }
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: TOOL_BASH.into(),
            description: "Run a shell command in the workspace. Only commands allowed by the \
                          operator's policy run; others are refused."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "timeout": { "type": "integer", "description": "seconds, optional" }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
    ]
}

/// Executes the slice-1 file tools against a single workspace root, enforcing the
/// `file store` path policy (no absolute/`..` escape; optional read/write globs).
pub struct FileToolExecutor {
    root: PathBuf,
    store_name: String,
    allow_read: Vec<String>,
    allow_write: Vec<String>,
    bash_allow: Vec<String>,
    tracker_queue: Option<String>,
    holder: String,
    max_bytes: usize,
}

impl FileToolExecutor {
    /// A workspace-rooted executor. Empty glob lists apply only the
    /// absolute/`..`-escape guard (the basic slice-1 sandbox); the `file store`
    /// glob policy is a slice-2 refinement. `bash` is default-deny: the allow-list
    /// of command prefixes comes from `WHIPPLESCRIPT_HARNESS_BASH_ALLOW`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            store_name: "workspace".to_string(),
            allow_read: Vec::new(),
            allow_write: Vec::new(),
            bash_allow: bash_allow_from_env(),
            tracker_queue: None,
            holder: "agent".to_string(),
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }

    /// Enable the tracker tools against a queue, attributing writes to `holder`
    /// (so `list_todos` can show agent- vs rule-filed items). Without this the
    /// tracker tools are refused (default-deny).
    pub fn with_tracker(mut self, queue: impl Into<String>, holder: impl Into<String>) -> Self {
        self.tracker_queue = Some(queue.into());
        self.holder = holder.into();
        self
    }

    // Wired to a source-declared `file store` policy in slice 2 (the governance
    // envelope); slice 1 only exercises it from tests, hence the allow.
    #[allow(dead_code)]
    pub fn with_policy(
        mut self,
        store_name: impl Into<String>,
        allow_read: Vec<String>,
        allow_write: Vec<String>,
    ) -> Self {
        self.store_name = store_name.into();
        self.allow_read = allow_read;
        self.allow_write = allow_write;
        self
    }

    fn policy(&self, path: &str, op: &str) -> Option<String> {
        let globs = if op == "write" {
            &self.allow_write
        } else {
            &self.allow_read
        };
        crate::file_path_policy_error(path, &self.store_name, globs, op)
    }

    /// Override the bash allow-list (test/programmatic use).
    #[allow(dead_code)]
    pub fn with_bash_allow(mut self, allow: Vec<String>) -> Self {
        self.bash_allow = allow;
        self
    }

    fn dispatch(&self, call: &ToolCall) -> Result<String, String> {
        let args = &call.arguments;
        match call.name.as_str() {
            TOOL_LIST_TODOS => self.list_todos(args),
            TOOL_ADD_TODO => self.add_todo(args),
            TOOL_UPDATE_TODO => self.update_todo(args),
            TOOL_BASH => self.bash(args),
            TOOL_READ => self.read(args),
            TOOL_WRITE => self.write(args),
            TOOL_EDIT => self.edit(args),
            TOOL_GREP => self.grep(args),
            TOOL_FIND => self.find(args),
            TOOL_LS => self.ls(args),
            other => Err(format!("unknown tool `{other}`")),
        }
    }

    fn read(&self, args: &Value) -> Result<String, String> {
        let path = str_arg(args, "path")?;
        if let Some(reason) = self.policy(path, "read") {
            return Err(reason);
        }
        let content = std::fs::read_to_string(self.root.join(path))
            .map_err(|e| format!("read of `{path}` failed: {e}"))?;
        let offset = usize_arg(args, "offset");
        let limit = usize_arg(args, "limit");
        let sliced = slice_lines(&content, offset, limit);
        Ok(bound(&sliced, self.max_bytes))
    }

    fn write(&self, args: &Value) -> Result<String, String> {
        let path = str_arg(args, "path")?;
        let content = str_arg(args, "content")?;
        if let Some(reason) = self.policy(path, "write") {
            return Err(reason);
        }
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("creating dirs for `{path}` failed: {e}"))?;
        }
        std::fs::write(&full, content).map_err(|e| format!("write of `{path}` failed: {e}"))?;
        Ok(format!("wrote {} bytes to {path}", content.len()))
    }

    fn edit(&self, args: &Value) -> Result<String, String> {
        let path = str_arg(args, "path")?;
        if let Some(reason) = self.policy(path, "write") {
            return Err(reason);
        }
        let edits = args
            .get("edits")
            .and_then(Value::as_array)
            .ok_or_else(|| "`edits` must be an array".to_string())?;
        let full = self.root.join(path);
        let mut content =
            std::fs::read_to_string(&full).map_err(|e| format!("read of `{path}` failed: {e}"))?;
        let mut applied = 0usize;
        for (index, edit) in edits.iter().enumerate() {
            let old = str_arg(edit, "oldText")?;
            let new = str_arg(edit, "newText")?;
            let matches = content.matches(old).count();
            if matches == 0 {
                return Err(format!("edit {index}: oldText not found in `{path}`"));
            }
            if matches > 1 {
                return Err(format!(
                    "edit {index}: oldText matches {matches} times in `{path}`; make it unique"
                ));
            }
            content = content.replacen(old, new, 1);
            applied += 1;
        }
        std::fs::write(&full, &content).map_err(|e| format!("write of `{path}` failed: {e}"))?;
        Ok(format!("applied {applied} edit(s) to {path}"))
    }

    fn ls(&self, args: &Value) -> Result<String, String> {
        let path = optional_str_arg(args, "path").unwrap_or(".");
        if let Some(reason) = self.policy(path, "read") {
            return Err(reason);
        }
        let limit = usize_arg(args, "limit").unwrap_or(500);
        let dir = self.root.join(path);
        let mut entries: Vec<String> = std::fs::read_dir(&dir)
            .map_err(|e| format!("ls of `{path}` failed: {e}"))?
            .filter_map(Result::ok)
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.path().is_dir() {
                    format!("{name}/")
                } else {
                    name
                }
            })
            .collect();
        entries.sort();
        entries.truncate(limit);
        Ok(entries.join("\n"))
    }

    fn find(&self, args: &Value) -> Result<String, String> {
        let pattern = str_arg(args, "pattern")?;
        let base = optional_str_arg(args, "path").unwrap_or(".");
        if let Some(reason) = self.policy(base, "read") {
            return Err(reason);
        }
        let limit = usize_arg(args, "limit").unwrap_or(1000);
        let mut hits = Vec::new();
        let mut walked = 0usize;
        walk(&self.root, &self.root.join(base), &mut walked, &mut |rel| {
            if crate::glob_match(pattern, rel) {
                hits.push(rel.to_string());
            }
        });
        hits.sort();
        hits.truncate(limit);
        if hits.is_empty() {
            Ok("No files found".to_string())
        } else {
            Ok(bound(&hits.join("\n"), self.max_bytes))
        }
    }

    fn grep(&self, args: &Value) -> Result<String, String> {
        let pattern = str_arg(args, "pattern")?;
        let base = optional_str_arg(args, "path").unwrap_or(".");
        if let Some(reason) = self.policy(base, "read") {
            return Err(reason);
        }
        let ignore_case = args
            .get("ignoreCase")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let needle = if ignore_case {
            pattern.to_lowercase()
        } else {
            pattern.to_string()
        };
        let limit = usize_arg(args, "limit").unwrap_or(100);
        let mut hits: Vec<String> = Vec::new();
        let root = self.root.clone();
        let mut walked = 0usize;
        walk(&root, &root.join(base), &mut walked, &mut |rel| {
            if hits.len() >= limit {
                return;
            }
            let Ok(content) = std::fs::read_to_string(root.join(rel)) else {
                return;
            };
            for (lineno, line) in content.lines().enumerate() {
                let haystack = if ignore_case {
                    line.to_lowercase()
                } else {
                    line.to_string()
                };
                if haystack.contains(&needle) {
                    hits.push(format!("{rel}:{}:{line}", lineno + 1));
                    if hits.len() >= limit {
                        break;
                    }
                }
            }
        });
        if hits.is_empty() {
            Ok("No matches".to_string())
        } else {
            Ok(bound(&hits.join("\n"), self.max_bytes))
        }
    }

    /// Run a shell command in the workspace. Default-deny: the command must match
    /// an allow-list prefix or it is refused (the sandbox boundary). Output is
    /// combined stdout+stderr, truncated; a non-zero exit is an error result.
    fn bash(&self, args: &Value) -> Result<String, String> {
        let command = str_arg(args, "command")?;
        if !self.command_allowed(command) {
            return Err(format!(
                "command refused: `{command}` is not permitted by WHIPPLESCRIPT_HARNESS_BASH_ALLOW"
            ));
        }
        let timeout = std::time::Duration::from_secs(
            args.get("timeout")
                .and_then(Value::as_u64)
                .unwrap_or(BASH_DEFAULT_TIMEOUT_SECS),
        );
        let output = run_bounded_command(command, &self.root, timeout)?;
        let combined = bound(&output.combined, self.max_bytes);
        match output.exit_code {
            Some(0) => Ok(combined),
            Some(code) => Err(format!("command exited with status {code}\n{combined}")),
            None => Err(format!("command terminated by signal\n{combined}")),
        }
    }

    /// A command is allowed if it equals an allow-list prefix or begins with one
    /// followed by whitespace (so `git` permits `git status` but not `gitfoo`).
    fn command_allowed(&self, command: &str) -> bool {
        let command = command.trim();
        self.bash_allow.iter().any(|prefix| {
            let prefix = prefix.trim();
            !prefix.is_empty()
                && (command == prefix
                    || command
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with(char::is_whitespace)))
        })
    }

    fn tracker(&self) -> Result<(WorkItemStore, String), String> {
        let queue = self.tracker_queue.clone().ok_or_else(|| {
            "tracker tools are not enabled for this turn (no tracker configured)".to_string()
        })?;
        let store = WorkItemStore::open(crate::items_store_path())
            .map_err(|error| format!("tracker store: {error:?}"))?;
        Ok((store, queue))
    }

    /// File a new tracker item (shared-state participation, refined I3): produces
    /// durable tracker state the workflow may observe, never a rule-matchable fact.
    fn add_todo(&self, args: &Value) -> Result<String, String> {
        let content = str_arg(args, "content")?;
        let (mut store, queue) = self.tracker()?;
        let holder = format!("agent:{}", self.holder);
        let item = store
            .file_item(&queue, content, "", &[], &json!({}), Some(&holder))
            .map_err(|error| format!("file_item: {error:?}"))?;
        Ok(json!({ "id": item.id }).to_string())
    }

    fn list_todos(&self, args: &Value) -> Result<String, String> {
        let (store, queue) = self.tracker()?;
        let status_filter = args
            .get("status")
            .and_then(Value::as_str)
            .map(todo_to_item_status);
        let items = store
            .list_items(Some(&queue), status_filter.as_deref())
            .map_err(|error| format!("list_items: {error:?}"))?;
        let rows: Vec<Value> = items
            .iter()
            .map(|item| {
                json!({
                    "id": item.id,
                    "content": item.title,
                    "status": item_to_todo_status(&item.status),
                    "source": if item.filed_by.as_deref().is_some_and(|f| f.starts_with("agent")) {
                        "agent"
                    } else {
                        "rule"
                    },
                })
            })
            .collect();
        Ok(Value::Array(rows).to_string())
    }

    fn update_todo(&self, args: &Value) -> Result<String, String> {
        let id = str_arg(args, "id")?;
        let status = str_arg(args, "status")?;
        let (mut store, _queue) = self.tracker()?;
        let holder = format!("agent:{}", self.holder);
        match status {
            "in_progress" => {
                store
                    .claim_item(id, &holder)
                    .map_err(|error| format!("claim: {error:?}"))?;
            }
            "completed" => {
                store
                    .finish_item(id, None)
                    .map_err(|error| format!("finish: {error:?}"))?;
            }
            "pending" => {
                store
                    .release_item(id)
                    .map_err(|error| format!("release: {error:?}"))?;
            }
            other => return Err(format!("unknown status `{other}`")),
        }
        Ok(json!({ "id": id, "status": status }).to_string())
    }
}

/// Map a TodoWrite-style status to the builtin tracker's item status.
fn todo_to_item_status(todo: &str) -> String {
    match todo {
        "pending" => "open",
        "in_progress" => "in_progress",
        "completed" => "done",
        other => other,
    }
    .to_string()
}

/// Map a tracker item status back to the TodoWrite-style status.
fn item_to_todo_status(item: &str) -> &'static str {
    match item {
        "in_progress" => "in_progress",
        "done" | "cancelled" => "completed",
        _ => "pending",
    }
}

/// Parse the bash allow-list from `WHIPPLESCRIPT_HARNESS_BASH_ALLOW` (comma- or
/// newline-separated command prefixes). Unset/empty = deny all (the default).
fn bash_allow_from_env() -> Vec<String> {
    std::env::var("WHIPPLESCRIPT_HARNESS_BASH_ALLOW")
        .ok()
        .map(|raw| {
            raw.split([',', '\n'])
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

struct CommandOutput {
    combined: String,
    exit_code: Option<i32>,
}

/// Run `command` via `sh -c` with `cwd = root`, killing it if it exceeds
/// `timeout`. Returns combined stdout+stderr and the exit code.
fn run_bounded_command(
    command: &str,
    root: &Path,
    timeout: std::time::Duration,
) -> Result<CommandOutput, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to spawn command: {error}"))?;

    let start = std::time::Instant::now();
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "command exceeded the {}s timeout",
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(error) => return Err(format!("failed to wait on command: {error}")),
        }
    };

    let mut combined = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut combined);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut combined);
    }
    Ok(CommandOutput {
        combined,
        exit_code: exit_status.code(),
    })
}

impl ToolExecutor for FileToolExecutor {
    fn execute(&self, call: &ToolCall) -> ToolOutcome {
        match self.dispatch(call) {
            Ok(content) => ToolOutcome {
                status: ToolStatus::Ok,
                content,
            },
            Err(reason) => ToolOutcome {
                status: ToolStatus::Error,
                content: reason,
            },
        }
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required string argument `{key}`"))
}

fn optional_str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn usize_arg(args: &Value, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
}

/// Apply a 1-based line offset and a line limit to file content.
fn slice_lines(content: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    if offset.is_none() && limit.is_none() {
        return content.to_string();
    }
    let start = offset.unwrap_or(1).saturating_sub(1);
    let lines: Vec<&str> = content.lines().collect();
    let end = match limit {
        Some(limit) => (start + limit).min(lines.len()),
        None => lines.len(),
    };
    if start >= lines.len() {
        return String::new();
    }
    lines[start..end].join("\n")
}

/// Bound returned content to a byte budget, appending a truncation marker.
fn bound(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[truncated: showing {end} of {} bytes]",
        &text[..end],
        text.len()
    )
}

/// Recursively walk `dir` (under `root`), invoking `visit` with each file's
/// root-relative slash path. Bounded by [`MAX_FILES_WALKED`].
fn walk(root: &Path, dir: &Path, walked: &mut usize, visit: &mut dyn FnMut(&str)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    children.sort();
    for path in children {
        if *walked >= MAX_FILES_WALKED {
            return;
        }
        if path.is_dir() {
            walk(root, &path, walked, visit);
        } else {
            *walked += 1;
            if let Ok(rel) = path.strip_prefix(root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                visit(&rel);
            }
        }
    }
}

/// System prompt for the slice-1 owned harness.
const OWNED_SYSTEM_PROMPT: &str =
    "You are a coding agent. Use the provided file tools to do the task, then \
     reply with a short summary and no further tool calls.";

/// Default per-turn model-step budget (overridable via WHIPPLESCRIPT_HARNESS_MAX_STEPS).
const OWNED_MAX_STEPS: usize = 16;

/// TTL for the per-turn workspace lease, in seconds. Long enough for a turn;
/// expiry reclaims the workspace if a worker dies mid-turn.
const OWNED_LEASE_TTL_SECONDS: i64 = 1800;

/// A deterministic, credential-free model client for dev/CI — the owned-harness
/// analogue of the fixture provider. By default it completes immediately; setting
/// `WHIPPLESCRIPT_OWNED_FIXTURE_TOOL=<tool>:<path>` makes its first reply issue
/// one tool call (e.g. `read:README.md`) before completing, so the brokered
/// loop's tool path is exercised without a live model.
pub struct FixtureModelClient {
    tool: Option<(String, String, Value)>,
}

impl FixtureModelClient {
    pub fn from_env() -> Self {
        let tool = std::env::var("WHIPPLESCRIPT_OWNED_FIXTURE_TOOL")
            .ok()
            .and_then(|spec| {
                let (name, path) = spec.split_once(':')?;
                Some((
                    "fixture_call_1".to_string(),
                    name.to_string(),
                    json!({ "path": path }),
                ))
            });
        Self { tool }
    }
}

impl HarnessModelClient for FixtureModelClient {
    fn next(
        &self,
        messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelReply, HarnessModelError> {
        let already_acted = messages
            .iter()
            .any(|message| matches!(message, ChatMessage::Assistant { .. }));
        if let Some((id, name, args)) = &self.tool {
            if !already_acted {
                return Ok(ModelReply {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: args.clone(),
                    }],
                    usage: json!({ "output_tokens": 1 }),
                });
            }
        }
        Ok(ModelReply {
            text: "owned-harness fixture turn complete".to_string(),
            tool_calls: Vec::new(),
            usage: json!({ "output_tokens": 1 }),
        })
    }
}

/// The workspace root a brokered turn operates in: `WHIPPLESCRIPT_HARNESS_WORKSPACE`
/// if set, else the current directory. The FileToolExecutor's no-escape guard
/// bounds all tools to this root.
pub fn owned_workspace_root() -> PathBuf {
    std::env::var_os("WHIPPLESCRIPT_HARNESS_WORKSPACE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Resolved configuration for the live owned-harness model client. Mirrors the
/// coerce knobs but in the independent `WHIPPLESCRIPT_HARNESS_*` namespace.
struct HarnessModelConfig {
    provider: CoerceProvider,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u64,
    timeout: Duration,
}

/// Resolve the live model client config. `Ok(None)` means run the credential-free
/// fixture client (dev/CI default); `Err` means the provider was requested but
/// could not be configured (fail the turn rather than silently use the fixture).
fn resolve_harness_model_config() -> Result<Option<HarnessModelConfig>, String> {
    let Some(provider_name) = std::env::var("WHIPPLESCRIPT_HARNESS_PROVIDER")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let provider = match provider_name.as_str() {
        "openai" => CoerceProvider::OpenAi,
        "anthropic" => CoerceProvider::Anthropic,
        other => {
            return Err(format!(
            "unknown WHIPPLESCRIPT_HARNESS_PROVIDER `{other}` (expected `openai` or `anthropic`)"
        ))
        }
    };
    let (api_key, _source) = resolve_credential_with_source(provider).ok_or_else(|| {
        format!("WHIPPLESCRIPT_HARNESS_PROVIDER={provider_name} is set but no credential resolved")
    })?;
    let model = std::env::var("WHIPPLESCRIPT_HARNESS_MODEL")
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "WHIPPLESCRIPT_HARNESS_MODEL is required when WHIPPLESCRIPT_HARNESS_PROVIDER is set"
                .to_string()
        })?;
    let base_url = std::env::var("WHIPPLESCRIPT_HARNESS_BASE_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| provider.default_base_url().to_string());
    let max_tokens = std::env::var("WHIPPLESCRIPT_HARNESS_MAX_TOKENS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(4096);
    let timeout = Duration::from_secs(
        std::env::var("WHIPPLESCRIPT_HARNESS_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(120),
    );
    Ok(Some(HarnessModelConfig {
        provider,
        api_key,
        model,
        base_url,
        max_tokens,
        timeout,
    }))
}

/// Run one owned/brokered agent turn: file tools over the workspace root, settled
/// to a single terminal fact. Uses the live provider model client when
/// `WHIPPLESCRIPT_HARNESS_PROVIDER` is set (credential-gated), else the
/// deterministic fixture client so dev/CI need no credentials.
#[allow(clippy::too_many_arguments)]
pub fn run_owned_agent_turn(
    kernel: &mut RuntimeKernel,
    instance_id: &str,
    effect_id: &str,
    agent: &str,
    profile: Option<&str>,
    input_json: &str,
) -> StoreResult<StoredEvent> {
    // Resolve the model client before taking the workspace lease, so a config
    // error never leaks a held lease.
    let model_config = resolve_harness_model_config().map_err(StoreError::Conflict)?;
    let workspace = owned_workspace_root();
    let mut executor = FileToolExecutor::new(&workspace);
    let mut tools = file_tool_specs();
    // Tracker tools (slice 4): offered only when a tracker queue is configured.
    if let Some(queue) = std::env::var("WHIPPLESCRIPT_HARNESS_TRACKER")
        .ok()
        .filter(|value| !value.is_empty())
    {
        executor = executor.with_tracker(queue, instance_id);
        tools.extend(tracker_tool_specs());
    }
    let input = BrokeredTurnInput {
        system: OWNED_SYSTEM_PROMPT.to_string(),
        user: input_json.to_string(),
        tools,
        max_steps: owned_max_steps(),
        // The runner populates resume_from from any persisted transcript on
        // crash recovery (slice 6); a fresh turn starts empty.
        resume_from: Vec::new(),
    };
    let ctx = BrokeredTurnContext {
        instance_id,
        effect_id,
        agent,
        profile,
    };

    // Slice-2 envelope: hold a durable workspace lease for the turn so concurrent
    // owned turns coordinate on a shared workspace. A contended workspace blocks
    // (recoverable) rather than racing; a later worker pass runs it once free.
    let resource = "owned.workspace";
    let key = workspace.display().to_string();
    let mut coordination = CoordinationStore::open(crate::coordination_store_path())?;
    match coordination.try_acquire(resource, &key, 1, OWNED_LEASE_TTL_SECONDS, instance_id)? {
        AcquireOutcome::Held => {}
        AcquireOutcome::Contended { .. } => {
            return kernel.block_effect_binding(
                instance_id,
                effect_id,
                "workspace_lease",
                &format!("workspace `{key}` is held by another agent turn"),
            );
        }
    }
    drop(coordination);

    let result = match model_config {
        Some(config) => {
            let transport = UreqCoerceTransport::new(config.timeout);
            let client = RealHarnessModelClient::new(
                &transport,
                config.provider,
                config.api_key,
                config.model,
                config.base_url,
                config.max_tokens,
            );
            kernel.run_brokered_agent_turn(&ctx, &client, &executor, &input)
        }
        None => {
            let client = FixtureModelClient::from_env();
            kernel.run_brokered_agent_turn(&ctx, &client, &executor, &input)
        }
    };

    // Release the workspace lease on every terminal (success or failure), mirroring
    // release_holder_resources_on_terminal for effect-held coordination.
    if let Ok(mut coordination) = CoordinationStore::open(crate::coordination_store_path()) {
        let _ = coordination.release(resource, &key, instance_id);
    }

    result
}

/// The per-turn model-step budget (the loop's enforced bound). Configurable via
/// `WHIPPLESCRIPT_HARNESS_MAX_STEPS`; the model cannot exceed it.
fn owned_max_steps() -> usize {
    std::env::var("WHIPPLESCRIPT_HARNESS_MAX_STEPS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|steps| *steps > 0)
        .unwrap_or(OWNED_MAX_STEPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "whip-harness-tools-{nanos}-{:?}",
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp root");
        dir
    }

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: "c".into(),
            name: name.into(),
            arguments: args,
        }
    }

    #[test]
    fn write_then_read_round_trip() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root);
        let w = exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "a/b.txt", "content": "hello" }),
        ));
        assert_eq!(w.status, ToolStatus::Ok);
        let r = exec.execute(&call(TOOL_READ, json!({ "path": "a/b.txt" })));
        assert_eq!(r.status, ToolStatus::Ok);
        assert_eq!(r.content, "hello");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn edit_requires_unique_match() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root);
        exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "f.txt", "content": "x x" }),
        ));
        // Two matches -> error (anti-idempotent, model must disambiguate).
        let dup = exec.execute(&call(
            TOOL_EDIT,
            json!({ "path": "f.txt", "edits": [{ "oldText": "x", "newText": "y" }] }),
        ));
        assert_eq!(dup.status, ToolStatus::Error);
        assert!(dup.content.contains("matches 2 times"));
        // Unique match -> applied.
        let ok = exec.execute(&call(
            TOOL_EDIT,
            json!({ "path": "f.txt", "edits": [{ "oldText": "x x", "newText": "z" }] }),
        ));
        assert_eq!(ok.status, ToolStatus::Ok);
        let r = exec.execute(&call(TOOL_READ, json!({ "path": "f.txt" })));
        assert_eq!(r.content, "z");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn edit_missing_oldtext_is_informative_error() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root);
        exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "f.txt", "content": "abc" }),
        ));
        let miss = exec.execute(&call(
            TOOL_EDIT,
            json!({ "path": "f.txt", "edits": [{ "oldText": "zzz", "newText": "y" }] }),
        ));
        assert_eq!(miss.status, ToolStatus::Error);
        assert!(miss.content.contains("not found"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn path_escape_is_refused() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root);
        let up = exec.execute(&call(TOOL_READ, json!({ "path": "../secret" })));
        assert_eq!(up.status, ToolStatus::Error);
        assert!(up.content.contains("escapes"));
        let abs = exec.execute(&call(TOOL_READ, json!({ "path": "/etc/passwd" })));
        assert_eq!(abs.status, ToolStatus::Error);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn write_glob_policy_blocks_disallowed_path() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root).with_policy(
            "src",
            vec!["**".into()],
            vec!["src/**".into()],
        );
        let blocked = exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "secrets.txt", "content": "x" }),
        ));
        assert_eq!(blocked.status, ToolStatus::Error);
        assert!(blocked.content.contains("allow write"));
        let allowed = exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "src/x.txt", "content": "x" }),
        ));
        assert_eq!(allowed.status, ToolStatus::Ok);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn grep_and_find_and_ls() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root);
        exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "src/a.rs", "content": "fn main() {}\nlet x = 1;" }),
        ));
        exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "src/b.txt", "content": "nothing here" }),
        ));

        let g = exec.execute(&call(TOOL_GREP, json!({ "pattern": "fn main" })));
        assert_eq!(g.status, ToolStatus::Ok);
        assert!(g.content.contains("src/a.rs:1:fn main() {}"));

        let f = exec.execute(&call(TOOL_FIND, json!({ "pattern": "**/*.rs" })));
        assert_eq!(f.status, ToolStatus::Ok);
        assert!(f.content.contains("src/a.rs"));
        assert!(!f.content.contains("src/b.txt"));

        let l = exec.execute(&call(TOOL_LS, json!({ "path": "src" })));
        assert_eq!(l.status, ToolStatus::Ok);
        assert!(l.content.contains("a.rs"));
        assert!(l.content.contains("b.txt"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bash_default_deny_refuses_without_allow_list() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root).with_bash_allow(vec![]);
        let r = exec.execute(&call(TOOL_BASH, json!({ "command": "echo hi" })));
        assert_eq!(r.status, ToolStatus::Error);
        assert!(r.content.contains("refused"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bash_runs_an_allowed_command() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root).with_bash_allow(vec!["echo".into()]);
        let r = exec.execute(&call(TOOL_BASH, json!({ "command": "echo hello" })));
        assert_eq!(r.status, ToolStatus::Ok);
        assert!(r.content.contains("hello"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn tracker_tools_refused_without_configuration() {
        // Default-deny: without with_tracker (no WHIPPLESCRIPT_HARNESS_TRACKER),
        // the tracker tools are refused before touching any store.
        let root = temp_root();
        let exec = FileToolExecutor::new(&root);
        let r = exec.execute(&call(TOOL_ADD_TODO, json!({ "content": "do a thing" })));
        assert_eq!(r.status, ToolStatus::Error);
        assert!(r.content.contains("not enabled"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn todo_status_mapping_round_trips() {
        assert_eq!(todo_to_item_status("pending"), "open");
        assert_eq!(todo_to_item_status("in_progress"), "in_progress");
        assert_eq!(todo_to_item_status("completed"), "done");
        assert_eq!(item_to_todo_status("open"), "pending");
        assert_eq!(item_to_todo_status("in_progress"), "in_progress");
        assert_eq!(item_to_todo_status("done"), "completed");
        assert_eq!(item_to_todo_status("cancelled"), "completed");
    }

    #[test]
    fn bash_refuses_command_outside_the_allow_list() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root).with_bash_allow(vec!["echo".into()]);
        // A dangerous command that does NOT match the `echo` prefix is refused
        // before any execution.
        let r = exec.execute(&call(TOOL_BASH, json!({ "command": "rm -rf /" })));
        assert_eq!(r.status, ToolStatus::Error);
        assert!(r.content.contains("refused"));
        // And a near-miss that only shares a prefix substring is also refused.
        let r2 = exec.execute(&call(TOOL_BASH, json!({ "command": "echofoo bar" })));
        assert_eq!(r2.status, ToolStatus::Error);
        std::fs::remove_dir_all(&root).ok();
    }
}
