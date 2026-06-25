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

use serde_json::{json, Value};
use whipplescript_kernel::harness_loop::{
    ToolCall, ToolExecutor, ToolOutcome, ToolSpec, ToolStatus,
};

pub const TOOL_READ: &str = "read";
pub const TOOL_WRITE: &str = "write";
pub const TOOL_EDIT: &str = "edit";
pub const TOOL_GREP: &str = "grep";
pub const TOOL_FIND: &str = "find";
pub const TOOL_LS: &str = "ls";

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
    ]
}

/// Executes the slice-1 file tools against a single workspace root, enforcing the
/// `file store` path policy (no absolute/`..` escape; optional read/write globs).
pub struct FileToolExecutor {
    root: PathBuf,
    store_name: String,
    allow_read: Vec<String>,
    allow_write: Vec<String>,
    max_bytes: usize,
}

impl FileToolExecutor {
    /// A workspace-rooted executor. Empty glob lists apply only the
    /// absolute/`..`-escape guard (the basic slice-1 sandbox); the `file store`
    /// glob policy is a slice-2 refinement.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            store_name: "workspace".to_string(),
            allow_read: Vec::new(),
            allow_write: Vec::new(),
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }

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

    fn dispatch(&self, call: &ToolCall) -> Result<String, String> {
        let args = &call.arguments;
        match call.name.as_str() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "whip-harness-tools-{nanos}-{:?}",
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
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
}
