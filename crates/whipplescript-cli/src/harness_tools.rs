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
use whipplescript_kernel::coerce_native::{json_schema_for_type, CoerceProvider};
use whipplescript_kernel::context_assembly::{assemble, BundleKind, ContextBundle};
use whipplescript_kernel::harness_loop::{
    BrokeredTurnInput, ChatMessage, HarnessModelClient, HarnessModelError, ModelReply, ToolCall,
    ToolExecutor, ToolOutcome, ToolSpec, ToolStatus,
};
use whipplescript_kernel::harness_model::RealHarnessModelClient;
use whipplescript_kernel::{BrokeredTurnContext, RuntimeKernel};
use whipplescript_parser::IrWorkflowContractKind;
use whipplescript_store::coordination::{AcquireOutcome, CoordinationStore};
use whipplescript_store::items::WorkItemStore;
use whipplescript_store::{
    RegisteredProfilePolicy, SqliteStore, StoreError, StoreResult, StoredEvent,
};

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

const TRACKER_RESOURCE: &str = "tracker";

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

#[cfg(test)]
fn file_tool_specs_for_profile(profile: Option<&str>) -> Vec<ToolSpec> {
    let policy = HarnessProfilePolicy::for_profile(profile);
    file_tool_specs_for_policy(&policy)
}

fn file_tool_specs_for_policy(policy: &HarnessProfilePolicy) -> Vec<ToolSpec> {
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
    .into_iter()
    .filter(|spec| policy.allows_tool(&spec.name))
    .collect()
}

fn file_tool_specs_for_turn(
    policy: &HarnessProfilePolicy,
    access: &TurnToolAccess,
) -> Vec<ToolSpec> {
    let read_files = access.file.read_globs.is_some();
    let write_files = access.file.write_globs.is_some();
    file_tool_specs_for_policy(policy)
        .into_iter()
        .filter(|spec| match spec.name.as_str() {
            TOOL_READ | TOOL_GREP | TOOL_FIND | TOOL_LS => read_files,
            TOOL_WRITE => write_files,
            TOOL_EDIT => read_files && write_files,
            TOOL_BASH => access.command_run,
            _ => true,
        })
        .collect()
}

fn tracker_tool_specs_for_turn(
    policy: &HarnessProfilePolicy,
    access: &TurnToolAccess,
) -> Vec<ToolSpec> {
    tracker_tool_specs()
        .into_iter()
        .filter(|spec| match spec.name.as_str() {
            TOOL_LIST_TODOS => true,
            TOOL_ADD_TODO => policy.tracker_file && access.tracker.file,
            TOOL_UPDATE_TODO => policy.allows_tracker_update() && access.tracker.allows_update(),
            _ => true,
        })
        .collect()
}

fn workflow_tool_specs_for_policy(
    policy: &HarnessProfilePolicy,
    specs: Vec<ToolSpec>,
) -> Vec<ToolSpec> {
    if policy.workflow_invoke {
        specs
    } else {
        Vec::new()
    }
}

/// A registered `@tool` sub-workflow (DR-0025): the tool name the model sees, the
/// source file to start, and the workflow root within it. Invocation drives the
/// child synchronously to its terminal via the brokered `workflow.invoke` facade.
#[derive(Clone)]
pub struct WorkflowToolEntry {
    name: String,
    path: PathBuf,
    root: String,
    package_id: String,
}

/// Executes the slice-1 file tools against a single workspace root, enforcing the
/// `file store` path policy (no absolute/`..` escape; optional read/write globs).
pub struct FileToolExecutor {
    root: PathBuf,
    file_policy: Option<FileToolPolicy>,
    bash_allow: Vec<String>,
    profile_policy: HarnessProfilePolicy,
    tracker_queue: Option<String>,
    holder: String,
    max_bytes: usize,
    /// `None` means no turn-access policy was installed (direct/test executor);
    /// `Some(false)` is the live owned-turn default-deny command policy.
    command_run_granted: Option<bool>,
    /// `None` preserves direct/test executor behavior; live owned turns install
    /// `Some` so tracker mutations are bound to `with access to tracker { ... }`.
    tracker_access: Option<TurnTrackerAccess>,
    /// Registered `@tool` sub-workflows (DR-0025), dispatched synchronously.
    workflow_tools: Vec<WorkflowToolEntry>,
    /// Run-store path the sub-workflow child instances are created in. Set
    /// together with `workflow_tools`; `None` disables workflow-tool dispatch.
    store_path: Option<PathBuf>,
    /// Per-child iteration bound for the synchronous sub-workflow drive.
    max_child_iterations: usize,
    /// Work-unit root (DR-0025): the lease holder this turn runs under. Sub-workflow
    /// children inherit it so they share the root's workspace lease re-entrantly.
    work_unit: String,
    /// The parent turn's provider configuration, carried into sub-workflow drives
    /// so a `@tool` workflow's own effects run under the same provider (DR-0025).
    provider_ctx: Option<crate::SubworkflowProviderContext>,
    /// Skill activation (context-assembly Phase 2, Decision 3): map of catalogue
    /// `location` → the registered content-addressed body. A `read` of a skill
    /// location resolves here (the registry) rather than the filesystem, so the
    /// model reads the exact registered bytes — identical on native and the DO.
    skill_bodies: std::collections::HashMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileToolPolicy {
    store_name: String,
    read_globs: Option<Vec<String>>,
    write_globs: Option<Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TurnFileAccess {
    store_name: String,
    read_globs: Option<Vec<String>>,
    write_globs: Option<Vec<String>>,
}

impl TurnFileAccess {
    fn deny_all() -> Self {
        Self {
            store_name: "turn_access".to_owned(),
            read_globs: None,
            write_globs: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TurnToolAccess {
    file: TurnFileAccess,
    file_resources: Vec<String>,
    command_run: bool,
    tracker: TurnTrackerAccess,
}

impl TurnToolAccess {
    fn deny_all() -> Self {
        Self {
            file: TurnFileAccess::deny_all(),
            file_resources: Vec::new(),
            command_run: false,
            tracker: TurnTrackerAccess::deny_all(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TurnTrackerAccess {
    file: bool,
    claim: bool,
    finish: bool,
    release: bool,
}

impl TurnTrackerAccess {
    fn deny_all() -> Self {
        Self {
            file: false,
            claim: false,
            finish: false,
            release: false,
        }
    }

    fn grant_update(&mut self) {
        self.claim = true;
        self.finish = true;
        self.release = true;
    }

    fn grant_write(&mut self) {
        self.file = true;
        self.grant_update();
    }

    fn allows_update(&self) -> bool {
        self.claim || self.finish || self.release
    }

    fn allows_status(&self, status: &str) -> bool {
        match status {
            "in_progress" => self.claim,
            "completed" => self.finish,
            "pending" => self.release,
            _ => false,
        }
    }

    fn mutates(&self) -> bool {
        self.file || self.allows_update()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HarnessProfilePolicy {
    profile: Option<String>,
    read_files: bool,
    write_files: bool,
    bash: bool,
    tracker_file: bool,
    tracker_claim: bool,
    tracker_finish: bool,
    tracker_release: bool,
    workflow_invoke: bool,
}

impl HarnessProfilePolicy {
    fn permissive() -> Self {
        Self {
            profile: None,
            read_files: true,
            write_files: true,
            bash: true,
            tracker_file: true,
            tracker_claim: true,
            tracker_finish: true,
            tracker_release: true,
            workflow_invoke: true,
        }
    }

    fn for_profile(profile: Option<&str>) -> Self {
        match profile {
            Some("repo-reader") | Some("human-review") => Self {
                profile: profile.map(str::to_owned),
                read_files: true,
                write_files: false,
                bash: false,
                tracker_file: false,
                tracker_claim: false,
                tracker_finish: false,
                tracker_release: false,
                workflow_invoke: true,
            },
            Some("no-repo") | Some("internet-research") => Self {
                profile: profile.map(str::to_owned),
                read_files: false,
                write_files: false,
                bash: false,
                tracker_file: false,
                tracker_claim: false,
                tracker_finish: false,
                tracker_release: false,
                workflow_invoke: true,
            },
            Some("repo-writer") | Some("permissive") | Some("release-operator") => Self {
                profile: profile.map(str::to_owned),
                read_files: true,
                write_files: true,
                bash: true,
                tracker_file: true,
                tracker_claim: true,
                tracker_finish: true,
                tracker_release: true,
                workflow_invoke: true,
            },
            // Package-defined/custom profiles do not have a local tool-policy
            // vocabulary in the owned harness yet. Preserve the existing behavior
            // until the registry-backed profile policy lands.
            _ => Self::permissive(),
        }
    }

    fn for_profile_with_registry(
        profile: Option<&str>,
        registered: Option<&RegisteredProfilePolicy>,
    ) -> Self {
        let base = Self::for_profile(profile);
        let Some(registered) = registered else {
            return base;
        };
        base.intersect(&Self::from_registered_policy(profile, registered))
    }

    fn from_registered_policy(profile: Option<&str>, registered: &RegisteredProfilePolicy) -> Self {
        if registered.enforcement_mode == "audit" {
            return Self {
                profile: profile.map(str::to_owned),
                read_files: true,
                write_files: true,
                bash: true,
                tracker_file: true,
                tracker_claim: true,
                tracker_finish: true,
                tracker_release: true,
                workflow_invoke: true,
            };
        }
        let allows = |capability: &str| {
            registered
                .allowed_capabilities
                .iter()
                .any(|allowed| allowed == "*" || allowed == capability)
        };
        Self {
            profile: profile.map(str::to_owned),
            read_files: allows("repo.read"),
            write_files: allows("repo.write"),
            bash: allows("command.run"),
            tracker_file: allows("tracker.write") || allows("tracker.file"),
            tracker_claim: allows("tracker.write")
                || allows("tracker.update")
                || allows("tracker.claim"),
            tracker_finish: allows("tracker.write")
                || allows("tracker.update")
                || allows("tracker.finish"),
            tracker_release: allows("tracker.write")
                || allows("tracker.update")
                || allows("tracker.release"),
            workflow_invoke: allows("workflow.invoke"),
        }
    }

    fn from_required_capabilities(required: &[String]) -> Option<Self> {
        let mut policy = Self {
            profile: None,
            read_files: false,
            write_files: false,
            bash: false,
            tracker_file: false,
            tracker_claim: false,
            tracker_finish: false,
            tracker_release: false,
            workflow_invoke: false,
        };
        let mut recognized = false;
        for capability in required {
            match capability.as_str() {
                "repo.read" => {
                    recognized = true;
                    policy.read_files = true;
                }
                "repo.write" => {
                    recognized = true;
                    policy.write_files = true;
                }
                "command.run" => {
                    recognized = true;
                    policy.bash = true;
                }
                "tracker.file" => {
                    recognized = true;
                    policy.tracker_file = true;
                }
                "tracker.claim" => {
                    recognized = true;
                    policy.tracker_claim = true;
                }
                "tracker.finish" => {
                    recognized = true;
                    policy.tracker_finish = true;
                }
                "tracker.release" => {
                    recognized = true;
                    policy.tracker_release = true;
                }
                "tracker.update" => {
                    recognized = true;
                    policy.tracker_claim = true;
                    policy.tracker_finish = true;
                    policy.tracker_release = true;
                }
                "tracker.write" => {
                    recognized = true;
                    policy.tracker_file = true;
                    policy.tracker_claim = true;
                    policy.tracker_finish = true;
                    policy.tracker_release = true;
                }
                "workflow.invoke" => {
                    recognized = true;
                    policy.workflow_invoke = true;
                }
                _ => {}
            }
        }
        recognized.then_some(policy)
    }

    fn intersect(&self, other: &Self) -> Self {
        Self {
            profile: self.profile.clone().or_else(|| other.profile.clone()),
            read_files: self.read_files && other.read_files,
            write_files: self.write_files && other.write_files,
            bash: self.bash && other.bash,
            tracker_file: self.tracker_file && other.tracker_file,
            tracker_claim: self.tracker_claim && other.tracker_claim,
            tracker_finish: self.tracker_finish && other.tracker_finish,
            tracker_release: self.tracker_release && other.tracker_release,
            workflow_invoke: self.workflow_invoke && other.workflow_invoke,
        }
    }

    fn profile_name(&self) -> &str {
        self.profile.as_deref().unwrap_or("<unspecified>")
    }

    fn allows_tool(&self, tool: &str) -> bool {
        match tool {
            TOOL_READ | TOOL_GREP | TOOL_FIND | TOOL_LS => self.read_files,
            TOOL_WRITE | TOOL_EDIT => self.write_files,
            TOOL_BASH => self.bash,
            TOOL_ADD_TODO => self.tracker_file,
            TOOL_UPDATE_TODO => self.allows_tracker_update(),
            _ => true,
        }
    }

    fn allows_tracker_update(&self) -> bool {
        self.tracker_claim || self.tracker_finish || self.tracker_release
    }

    fn allows_tracker_status(&self, status: &str) -> bool {
        match status {
            "in_progress" => self.tracker_claim,
            "completed" => self.tracker_finish,
            "pending" => self.tracker_release,
            _ => false,
        }
    }
}

impl FileToolExecutor {
    /// A workspace-rooted executor. Empty glob lists apply only the
    /// absolute/`..`-escape guard (the basic slice-1 sandbox); the `file store`
    /// glob policy is a slice-2 refinement. `bash` is default-deny: the allow-list
    /// of command prefixes comes from `WHIPPLESCRIPT_HARNESS_BASH_ALLOW`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            file_policy: None,
            bash_allow: bash_allow_from_env(),
            profile_policy: HarnessProfilePolicy::permissive(),
            tracker_queue: None,
            holder: "agent".to_string(),
            max_bytes: DEFAULT_MAX_BYTES,
            command_run_granted: None,
            tracker_access: None,
            workflow_tools: Vec::new(),
            store_path: None,
            max_child_iterations: 8,
            work_unit: String::new(),
            provider_ctx: None,
            skill_bodies: std::collections::HashMap::new(),
        }
    }

    /// Install the skill activation registry: a map of catalogue `location` → the
    /// registered content-addressed body. A `read` of one of these locations
    /// resolves through the registry instead of the filesystem (Decision 3).
    pub fn with_skill_bodies(
        mut self,
        skill_bodies: std::collections::HashMap<String, String>,
    ) -> Self {
        self.skill_bodies = skill_bodies;
        self
    }

    /// Register `@tool` sub-workflows (DR-0025) for synchronous dispatch. The
    /// child instances are created in `store_path`; each tool call drives one
    /// child to its terminal (bounded by `max_child_iterations`) and returns its
    /// output payload. Without this, a workflow-tool call is an unknown tool.
    pub fn with_workflow_tools(
        mut self,
        workflow_tools: Vec<WorkflowToolEntry>,
        store_path: impl Into<PathBuf>,
        max_child_iterations: usize,
        work_unit: impl Into<String>,
        provider_ctx: crate::SubworkflowProviderContext,
    ) -> Self {
        self.workflow_tools = workflow_tools;
        self.store_path = Some(store_path.into());
        self.max_child_iterations = max_child_iterations.max(1);
        self.work_unit = work_unit.into();
        self.provider_ctx = Some(provider_ctx);
        self
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
        self.file_policy = Some(FileToolPolicy {
            store_name: store_name.into(),
            read_globs: Some(allow_read),
            write_globs: Some(allow_write),
        });
        self
    }

    #[cfg(test)]
    fn with_turn_file_access(mut self, access: TurnFileAccess) -> Self {
        self.file_policy = Some(FileToolPolicy {
            store_name: access.store_name,
            read_globs: access.read_globs,
            write_globs: access.write_globs,
        });
        self.command_run_granted = Some(false);
        self.tracker_access = Some(TurnTrackerAccess::deny_all());
        self
    }

    fn with_turn_tool_access(mut self, access: TurnToolAccess) -> Self {
        self.file_policy = Some(FileToolPolicy {
            store_name: access.file.store_name,
            read_globs: access.file.read_globs,
            write_globs: access.file.write_globs,
        });
        self.command_run_granted = Some(access.command_run);
        self.tracker_access = Some(access.tracker);
        self
    }

    #[cfg(test)]
    fn with_profile_policy(mut self, profile: Option<&str>) -> Self {
        self.profile_policy = HarnessProfilePolicy::for_profile(profile);
        self
    }

    fn with_resolved_profile_policy(mut self, policy: HarnessProfilePolicy) -> Self {
        self.profile_policy = policy;
        self
    }

    fn policy(&self, path: &str, op: &str) -> Option<String> {
        if op == "write" && !self.profile_policy.write_files {
            return Some(format!(
                "file write is not permitted by profile `{}`",
                self.profile_policy.profile_name()
            ));
        }
        if op != "write" && !self.profile_policy.read_files {
            return Some(format!(
                "file read is not permitted by profile `{}`",
                self.profile_policy.profile_name()
            ));
        }
        let Some(policy) = &self.file_policy else {
            return crate::file_path_policy_error(path, "workspace", &[], op);
        };
        let globs = if op == "write" {
            policy.write_globs.as_ref()
        } else {
            policy.read_globs.as_ref()
        };
        match globs {
            Some(globs) => crate::file_path_policy_error(path, &policy.store_name, globs, op),
            None => Some(format!(
                "file {op} is not granted for store `{}` in this turn",
                policy.store_name
            )),
        }
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
            other => match self.workflow_tools.iter().find(|tool| tool.name == other) {
                Some(tool) => self.invoke_workflow_tool(tool, args),
                None => Err(format!("unknown tool `{other}`")),
            },
        }
    }

    /// Synchronously run a `@tool` sub-workflow (DR-0025) and return its output.
    /// The child is convergence-checked at turn setup, so the drive is bounded;
    /// the tool call blocks the turn until the sub-workflow reaches its terminal.
    /// A non-`completed` terminal (failed/cancelled) surfaces as a tool error the
    /// model sees, never a silent success.
    fn invoke_workflow_tool(
        &self,
        tool: &WorkflowToolEntry,
        args: &Value,
    ) -> Result<String, String> {
        if !self.profile_policy.workflow_invoke {
            return Err(format!(
                "workflow tool invoke is not permitted by profile `{}`",
                self.profile_policy.profile_name()
            ));
        }
        let store_path = self.store_path.as_ref().ok_or_else(|| {
            "workflow tools are not enabled for this turn (no store configured)".to_string()
        })?;
        let provider_ctx = self.provider_ctx.as_ref().ok_or_else(|| {
            "workflow tools are not enabled for this turn (no provider context)".to_string()
        })?;
        let input_json = args.to_string();
        let summary = crate::drive_subworkflow_tool(
            store_path,
            &tool.path,
            &tool.root,
            &tool.package_id,
            &input_json,
            self.max_child_iterations,
            &self.work_unit,
            provider_ctx,
        )
        .map_err(|error| format!("sub-workflow `{}` failed to run: {error:?}", tool.name))?;
        match summary.status.as_str() {
            "completed" => Ok(summary.payload.to_string()),
            other => Err(format!(
                "sub-workflow `{}` terminated `{other}`: {}",
                tool.name, summary.payload
            )),
        }
    }

    fn read(&self, args: &Value) -> Result<String, String> {
        let path = str_arg(args, "path")?;
        // Skill activation (Decision 3): a read of a catalogue location resolves to
        // the registered content-addressed body from the registry, not the
        // filesystem — identical bytes on native and the durable object. The
        // catalogue is only offered alongside a read tool, so this activation is
        // authorized independently of the workspace file globs.
        if let Some(body) = self.skill_bodies.get(path) {
            let offset = usize_arg(args, "offset");
            let limit = usize_arg(args, "limit");
            let sliced = slice_lines(body, offset, limit);
            return Ok(bound(&sliced, self.max_bytes));
        }
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
        if let Some(reason) = self.policy(path, "read") {
            return Err(reason);
        }
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
        if !self.profile_policy.bash {
            return Err(format!(
                "bash is not permitted by profile `{}`",
                self.profile_policy.profile_name()
            ));
        }
        if self.command_run_granted == Some(false) {
            return Err(
                "bash is not granted for this turn (`with access to command { run }` required)"
                    .to_owned(),
            );
        }
        if !self.command_allowed(command) {
            return Err(format!(
                "command refused: `{command}` is not permitted by WHIPPLESCRIPT_HARNESS_BASH_ALLOW"
            ));
        }
        self.enforce_command_read_boundary(command)?;
        self.enforce_command_write_boundary(command)?;
        if let Some(reason) = command_argument_policy_violation(command) {
            return Err(format!("command refused: {reason}"));
        }
        self.enforce_command_path_argument_boundary(command)?;
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
        self.command_prefix_allowed(command)
    }

    fn command_prefix_allowed(&self, command: &str) -> bool {
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

    fn enforce_command_write_boundary(&self, command: &str) -> Result<(), String> {
        for target in command_output_redirection_targets(command)? {
            if is_fd_redirection_target(&target) || target == "/dev/null" {
                continue;
            }
            if target.contains(['$', '`', '*', '?', '[', ']', '{', '}', '~']) {
                return Err(format!(
                    "command output redirection target `{target}` must be a literal workspace-relative path"
                ));
            }
            if let Some(reason) = self.policy(&target, "write") {
                return Err(format!(
                    "command output redirection to `{target}` refused: {reason}"
                ));
            }
        }
        Ok(())
    }

    fn enforce_command_read_boundary(&self, command: &str) -> Result<(), String> {
        for target in command_input_redirection_targets(command)? {
            if target.contains(['$', '`', '*', '?', '[', ']', '{', '}', '~']) {
                return Err(format!(
                    "command input redirection target `{target}` must be a literal workspace-relative path"
                ));
            }
            if let Some(reason) = self.policy(&target, "read") {
                return Err(format!(
                    "command input redirection from `{target}` refused: {reason}"
                ));
            }
        }
        Ok(())
    }

    fn enforce_command_path_argument_boundary(&self, command: &str) -> Result<(), String> {
        let words = command_words(command)?;
        for word in &words {
            if let Some(reason) = command_path_argument_policy_violation(word) {
                return Err(format!("command path argument `{word}` refused: {reason}"));
            }
        }
        Ok(())
    }

    fn tracker(&self) -> Result<(WorkItemStore, String), String> {
        let queue = self.tracker_queue.clone().ok_or_else(|| {
            "tracker tools are not enabled for this turn (no tracker configured)".to_string()
        })?;
        let store = WorkItemStore::open(crate::items_store_path())
            .map_err(|error| format!("tracker store: {error:?}"))?;
        Ok((store, queue))
    }

    fn tracker_write_policy(&self, action: &str, status: Option<&str>) -> Option<String> {
        let profile_allows = match action {
            "file" => self.profile_policy.tracker_file,
            "update" => status
                .map(|status| self.profile_policy.allows_tracker_status(status))
                .unwrap_or_else(|| self.profile_policy.allows_tracker_update()),
            _ => true,
        };
        if !profile_allows {
            return Some(format!(
                "tracker {action} is not permitted by profile `{}`",
                self.profile_policy.profile_name()
            ));
        }
        let Some(access) = &self.tracker_access else {
            return None;
        };
        let granted = match action {
            "file" => access.file,
            "update" => status
                .map(|status| access.allows_status(status))
                .unwrap_or_else(|| access.allows_update()),
            _ => true,
        };
        if granted {
            None
        } else {
            let expected = match (action, status) {
                ("file", _) => "`with access to tracker { file }`",
                ("update", Some("in_progress")) => "`with access to tracker { claim }`",
                ("update", Some("completed")) => "`with access to tracker { finish }`",
                ("update", Some("pending")) => "`with access to tracker { release }`",
                ("update", _) => "`with access to tracker { update }`",
                _ => "`with access to tracker { write }`",
            };
            Some(format!(
                "tracker {action} is not granted for this turn ({expected} required)"
            ))
        }
    }

    /// File a new tracker item (shared-state participation, refined I3): produces
    /// durable tracker state the workflow may observe, never a rule-matchable fact.
    fn add_todo(&self, args: &Value) -> Result<String, String> {
        if let Some(reason) = self.tracker_write_policy("file", None) {
            return Err(reason);
        }
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
        if let Some(reason) = self.tracker_write_policy("update", Some(status)) {
            return Err(reason);
        }
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
        "completed" => "closed",
        other => other,
    }
    .to_string()
}

/// Map a tracker issue status back to the TodoWrite-style status.
fn item_to_todo_status(item: &str) -> &'static str {
    match item {
        "in_progress" => "in_progress",
        "closed" | "canceled" | "archived" => "completed",
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

fn command_argument_policy_violation(command: &str) -> Option<String> {
    let bytes = command.as_bytes();
    let mut index = 0usize;
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if single_quoted {
            if byte == b'\'' {
                single_quoted = false;
            }
            index += 1;
            continue;
        }
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if double_quoted {
            match byte {
                b'\\' => {
                    escaped = true;
                    index += 1;
                    continue;
                }
                b'"' => {
                    double_quoted = false;
                    index += 1;
                    continue;
                }
                b'`' => {
                    return Some("command substitution with backticks is not permitted".to_owned());
                }
                b'$' if bytes.get(index + 1).is_some_and(|next| *next == b'(') => {
                    return Some("command substitution with `$(` is not permitted".to_owned());
                }
                b'$' => {
                    return Some("shell variable expansion is not permitted".to_owned());
                }
                _ => {
                    index += 1;
                    continue;
                }
            }
        }
        match byte {
            b'\\' => escaped = true,
            b'\'' => single_quoted = true,
            b'"' => double_quoted = true,
            b'`' => {
                return Some("command substitution with backticks is not permitted".to_owned());
            }
            b'$' if bytes.get(index + 1).is_some_and(|next| *next == b'(') => {
                return Some("command substitution with `$(` is not permitted".to_owned());
            }
            b'$' => {
                return Some("shell variable expansion is not permitted".to_owned());
            }
            b'*' | b'?' => {
                return Some("shell glob expansion is not permitted".to_owned());
            }
            b'[' | b']' if !is_shell_test_bracket_delimiter(command, index, byte) => {
                return Some("shell glob expansion is not permitted".to_owned());
            }
            b'{' | b'}' => {
                return Some("shell brace expansion is not permitted".to_owned());
            }
            b'~' => {
                return Some("shell tilde expansion is not permitted".to_owned());
            }
            b';' | b'|' | b'&' | b'(' | b')' => {
                return Some(format!(
                    "shell control operator `{}` is not permitted",
                    byte as char
                ));
            }
            b'\n' | b'\r' => {
                if command[index..].trim().is_empty() {
                    break;
                }
                return Some("shell command separators are not permitted".to_owned());
            }
            _ => {}
        }
        index += 1;
    }
    if single_quoted || double_quoted {
        return Some("command has an unterminated quote".to_owned());
    }
    if escaped {
        return Some("command has a trailing escape".to_owned());
    }
    None
}

fn is_shell_test_bracket_delimiter(command: &str, index: usize, byte: u8) -> bool {
    let bytes = command.as_bytes();
    match byte {
        b'[' => {
            command[..index].trim().is_empty()
                && bytes
                    .get(index + 1)
                    .is_some_and(|next| next.is_ascii_whitespace())
        }
        b']' => {
            command[index + 1..].trim().is_empty()
                && command.trim_start().starts_with("[ ")
                && index > 0
                && bytes[index - 1].is_ascii_whitespace()
        }
        _ => false,
    }
}

fn command_words(command: &str) -> Result<Vec<String>, String> {
    let bytes = command.as_bytes();
    let mut words = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        if matches!(bytes[index], b'<' | b'>') {
            index += 1;
            while index < bytes.len() && matches!(bytes[index], b'<' | b'>' | b'|') {
                index += 1;
            }
            let (_, next_index) = shell_word_at(command, index)?;
            index = next_index;
            continue;
        }
        let (word, next_index) = shell_word_at(command, index)?;
        match word {
            Some(word) => {
                words.push(word);
                index = next_index;
            }
            None => {
                index = index.saturating_add(1);
            }
        }
    }
    Ok(words)
}

fn command_path_argument_policy_violation(word: &str) -> Option<String> {
    if path_argument_escapes_workspace(word) {
        return Some("must stay within the workspace".to_owned());
    }
    if let Some((_, value)) = word.split_once('=') {
        if path_argument_escapes_workspace(value) {
            return Some("must stay within the workspace".to_owned());
        }
    }
    None
}

fn path_argument_escapes_workspace(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if value == "~" || value.starts_with("~/") || value.starts_with('/') {
        return true;
    }
    Path::new(value)
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
}

fn command_output_redirection_targets(command: &str) -> Result<Vec<String>, String> {
    let bytes = command.as_bytes();
    let mut targets = Vec::new();
    let mut index = 0usize;
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if single_quoted {
            if byte == b'\'' {
                single_quoted = false;
            }
            index += 1;
            continue;
        }
        if double_quoted {
            if escaped {
                escaped = false;
                index += 1;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => double_quoted = false,
                _ => {}
            }
            index += 1;
            continue;
        }
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match byte {
            b'\\' => {
                escaped = true;
                index += 1;
            }
            b'\'' => {
                single_quoted = true;
                index += 1;
            }
            b'"' => {
                double_quoted = true;
                index += 1;
            }
            b'>' => {
                let mut target_start = index + 1;
                if bytes
                    .get(target_start)
                    .is_some_and(|next| *next == b'>' || *next == b'|')
                {
                    target_start += 1;
                }
                let (target, next_index) = shell_word_at(command, target_start)?;
                let Some(target) = target else {
                    return Err("command output redirection is missing a target path".to_owned());
                };
                targets.push(target);
                index = next_index;
            }
            _ => index += 1,
        }
    }
    Ok(targets)
}

fn command_input_redirection_targets(command: &str) -> Result<Vec<String>, String> {
    let bytes = command.as_bytes();
    let mut targets = Vec::new();
    let mut index = 0usize;
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if single_quoted {
            if byte == b'\'' {
                single_quoted = false;
            }
            index += 1;
            continue;
        }
        if double_quoted {
            if escaped {
                escaped = false;
                index += 1;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => double_quoted = false,
                _ => {}
            }
            index += 1;
            continue;
        }
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match byte {
            b'\\' => {
                escaped = true;
                index += 1;
            }
            b'\'' => {
                single_quoted = true;
                index += 1;
            }
            b'"' => {
                double_quoted = true;
                index += 1;
            }
            b'<' => {
                let mut target_start = index + 1;
                match bytes.get(target_start) {
                    // Here-doc and here-string redirections do not name a file.
                    Some(b'<') => {
                        index += 2;
                        continue;
                    }
                    Some(b'>') => {
                        // Read/write redirection (`<> path`): this function
                        // enforces the read half; the output scanner enforces
                        // the write half.
                        target_start += 1;
                    }
                    _ => {}
                }
                let (target, next_index) = shell_word_at(command, target_start)?;
                let Some(target) = target else {
                    return Err("command input redirection is missing a target path".to_owned());
                };
                targets.push(target);
                index = next_index;
            }
            _ => index += 1,
        }
    }
    Ok(targets)
}

fn shell_word_at(command: &str, start: usize) -> Result<(Option<String>, usize), String> {
    let bytes = command.as_bytes();
    let mut index = start;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if index >= bytes.len() {
        return Ok((None, index));
    }
    let mut word = String::new();
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if single_quoted {
            if byte == b'\'' {
                single_quoted = false;
            } else {
                word.push(byte as char);
            }
            index += 1;
            continue;
        }
        if double_quoted {
            if escaped {
                word.push(byte as char);
                escaped = false;
                index += 1;
                continue;
            }
            match byte {
                b'\\' => escaped = true,
                b'"' => double_quoted = false,
                _ => word.push(byte as char),
            }
            index += 1;
            continue;
        }
        if escaped {
            word.push(byte as char);
            escaped = false;
            index += 1;
            continue;
        }
        if byte.is_ascii_whitespace() || matches!(byte, b';' | b'|' | b'<') {
            break;
        }
        match byte {
            b'\\' => escaped = true,
            b'\'' => single_quoted = true,
            b'"' => double_quoted = true,
            _ => word.push(byte as char),
        }
        index += 1;
    }
    if single_quoted || double_quoted {
        return Err("command output redirection target has an unterminated quote".to_owned());
    }
    if word.is_empty() {
        Ok((None, index))
    } else {
        Ok((Some(word), index))
    }
}

fn is_fd_redirection_target(target: &str) -> bool {
    target
        .strip_prefix('&')
        .is_some_and(|rest| rest == "-" || rest.chars().all(|ch| ch.is_ascii_digit()))
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

/// Persona bundle for the owned harness. Mirrors pi's persona shape, adapted to
/// the WhippleScript brokered harness, and folds in the turn-scoped authority the
/// loop relies on. Termination guidance lives in [`OWNED_GUIDELINES`].
const OWNED_PERSONA: &str = "You are an expert coding assistant operating inside the \
WhippleScript owned agent harness. You help by reading files, running commands, \
editing code, and writing new files. Use only the provided tools and the authority \
granted for this turn to do the task.";

/// Guidelines bundle lines. The first two mirror pi's always-on guidelines; the
/// last two carry the owned-loop contract (only the provided tools; the turn ends
/// when the model stops calling tools).
const OWNED_GUIDELINES: &[&str] = &[
    "Be concise in your responses.",
    "Show file paths clearly when working with files.",
    "Use only the tools provided for this turn; do not assume tools you were not given.",
    "When finished, reply with a short summary and make no further tool calls.",
];

/// The owned-harness system-prompt bundles in pi's order: persona, one-line tool
/// snippets, guidelines, current date, current working directory. Project-context
/// and available-skills slots are populated in later tracker phases. The host
/// supplies `date`/`cwd` (kept out of the pure kernel assembler).
/// One entry in the `<available_skills>` catalogue: what the model needs to decide
/// relevance and where to read the full instructions (Decision 2, discover-all).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillCatalogueEntry {
    pub name: String,
    pub description: String,
    pub location: String,
}

/// Whether the turn has a read-class tool the model can use to load a skill body.
/// Without one the catalogue is pointless (nothing can fetch the SKILL.md).
fn has_read_class_tool(tools: &[ToolSpec]) -> bool {
    tools.iter().any(|tool| tool.name == TOOL_READ)
}

/// Render the `<available_skills>` catalogue bundle body: one entry per skill with
/// its `name`, `description`, and `location` (the model reads the location on
/// demand to activate a skill — an ordinary, evidence-logged tool call).
fn render_available_skills(skills: &[SkillCatalogueEntry]) -> String {
    let mut body = String::from(
        "Available skills — read a skill's location to load its full instructions:\n<available_skills>",
    );
    for skill in skills {
        body.push_str(&format!(
            "\n  <skill name=\"{}\" location=\"{}\">\n  {}\n  </skill>",
            skill.name, skill.location, skill.description
        ));
    }
    body.push_str("\n</available_skills>");
    body
}

fn owned_context_bundles(
    tools: &[ToolSpec],
    date: &str,
    cwd: &str,
    skills: &[SkillCatalogueEntry],
    project_instructions: &[crate::project_context::ProjectInstruction],
) -> Vec<ContextBundle> {
    let mut bundles = vec![ContextBundle::new(
        BundleKind::Persona,
        "builtin:persona",
        "v1",
        OWNED_PERSONA,
    )];

    if !tools.is_empty() {
        let mut body = String::from("Available tools:\n");
        for tool in tools {
            body.push_str(&format!(
                "- {}: {}\n",
                tool.name,
                first_line(&tool.description)
            ));
        }
        bundles.push(ContextBundle::new(
            BundleKind::Tools,
            "builtin:tools",
            "v1",
            body.trim_end(),
        ));
    }

    let mut guidelines = String::from("Guidelines:\n");
    for line in OWNED_GUIDELINES {
        guidelines.push_str(&format!("- {line}\n"));
    }
    bundles.push(ContextBundle::new(
        BundleKind::Guidelines,
        "builtin:guidelines",
        "v1",
        guidelines.trim_end(),
    ));

    bundles.push(ContextBundle::new(
        BundleKind::Date,
        "host:clock",
        "v1",
        format!("Current date: {date}"),
    ));
    bundles.push(ContextBundle::new(
        BundleKind::Cwd,
        "host:cwd",
        "v1",
        format!("Current working directory: {cwd}"),
    ));

    // Project instructions (AGENTS.md / CLAUDE.md), injected verbatim wrapped in
    // `<project_context>` (context-assembly Phase 3). The host discovers them.
    if !project_instructions.is_empty() {
        bundles.push(ContextBundle::new(
            BundleKind::ProjectContext,
            "fs:project-instructions",
            "v1",
            crate::project_context::render_project_context(project_instructions),
        ));
    }

    // The `<available_skills>` catalogue (Decision 2: discover-all). Only when a
    // read-class tool is present — otherwise the model cannot load a skill body.
    // The assembler renders this in its canonical slot regardless of push order.
    if !skills.is_empty() && has_read_class_tool(tools) {
        bundles.push(ContextBundle::new(
            BundleKind::AvailableSkills,
            "registry:skills",
            "v1",
            render_available_skills(skills),
        ));
    }

    bundles
}

/// The first non-empty line of a tool description, for the one-line prompt snippet.
fn first_line(description: &str) -> &str {
    description
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

/// The current UTC date as `YYYY-MM-DD` for the date bundle. Date-only (not
/// time-of-day) keeps the assembled prefix stable within a day, which is a
/// prompt-cache technique, not just cosmetics.
fn owned_context_date() -> String {
    // The CLI's chrono is built without the `clock` feature, so derive the date
    // from the system clock via a UNIX timestamp (pure arithmetic, no `clock`).
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|delta| delta.as_secs())
        .unwrap_or(0);
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

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
                let (name, rest) = spec.split_once(':')?;
                // `tool:{json}` passes the JSON object as the call arguments
                // verbatim (used for workflow tools, whose input is structured);
                // `tool:value` is the shorthand for a file tool's `{ "path": value }`.
                let arguments = match serde_json::from_str::<Value>(rest) {
                    Ok(value @ Value::Object(_)) => value,
                    _ => json!({ "path": rest }),
                };
                Some(("fixture_call_1".to_string(), name.to_string(), arguments))
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

/// Build the model-facing tool spec and the dispatch entry for one resolved
/// `@tool` workflow (DR-0025): the tool name is the workflow name, its declared
/// `input` contract is the JSON schema, its `description` (if any) the tool blurb,
/// and `source_path`+root tell the dispatcher how to drive it.
fn tool_spec_and_entry(
    ir: &whipplescript_parser::IrProgram,
    source_path: PathBuf,
    package_id: String,
) -> (ToolSpec, WorkflowToolEntry) {
    let input_schema = ir
        .workflow_contracts
        .iter()
        .find(|contract| contract.kind == IrWorkflowContractKind::Input)
        .map(|contract| json_schema_for_type(&contract.ty, &ir.schemas))
        .unwrap_or_else(|| json!({ "type": "object", "additionalProperties": false }));
    let description = ir
        .source_descriptions
        .iter()
        .find(|desc| desc.target_kind == "workflow" && desc.target == ir.workflow)
        .map(|desc| desc.value.clone())
        .unwrap_or_else(|| {
            format!(
                "Run the `{}` sub-workflow synchronously and return its output.",
                ir.workflow
            )
        });
    (
        ToolSpec {
            name: ir.workflow.clone(),
            description,
            input_schema,
        },
        WorkflowToolEntry {
            name: ir.workflow.clone(),
            path: source_path,
            root: ir.workflow.clone(),
            package_id,
        },
    )
}

/// Discover `@tool` sub-workflows (DR-0025) from `WHIPPLESCRIPT_HARNESS_TOOLS`
/// (comma/newline-separated source paths). This is the operator-level override
/// for out-of-tree tool files; in-program curation is the per-agent `tools` grant
/// (see [`load_agent_granted_tools`]). Each path is compiled *for validation* —
/// running the convergence lint — so a non-`@tool` or non-convergent file fails
/// the turn at setup rather than blocking it mid-run.
fn load_workflow_tools() -> Result<(Vec<ToolSpec>, Vec<WorkflowToolEntry>), String> {
    let Some(raw) = std::env::var("WHIPPLESCRIPT_HARNESS_TOOLS")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut specs = Vec::new();
    let mut entries = Vec::new();
    for path in raw
        .split([',', '\n'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let (_, ir) = crate::compile_source_path_for_validation(path, None)
            .map_err(|error| crate::child_compile_error(path, error))?;
        let is_tool = ir.source_tags.iter().any(|tag| {
            tag.target_kind == "workflow" && tag.target == ir.workflow && tag.name == "tool"
        });
        if !is_tool {
            return Err(format!(
                "workflow-tool file `{path}` declares `{}`, which is not tagged `@tool`",
                ir.workflow
            ));
        }
        let (spec, entry) = tool_spec_and_entry(
            &ir,
            PathBuf::from(path),
            crate::LOCAL_WORKFLOW_PACKAGE.to_owned(),
        );
        specs.push(spec);
        entries.push(entry);
    }
    Ok((specs, entries))
}

/// Resolve the `tools [...]` grant of the agent running this turn (DR-0025): the
/// in-program curation surface. Each granted name is resolved to a convergence-
/// eligible `@tool` workflow (same bundle, or a `use`d package) and turned into a
/// typed tool. An unresolvable or non-`@tool` grant fails the turn at setup — the
/// same condition `whip check` rejects statically. Returns empty if the program/
/// agent context is unavailable (e.g. an ad-hoc turn) or the agent grants nothing.
fn load_agent_granted_tools(
    program_path: Option<&Path>,
    root: Option<&str>,
    agent: &str,
    package_lock_path: Option<&Path>,
) -> Result<(Vec<ToolSpec>, Vec<WorkflowToolEntry>), String> {
    let Some(program_path) = program_path else {
        return Ok((Vec::new(), Vec::new()));
    };
    let (_, ir) =
        crate::compile_source_path_with_root(program_path.to_str().unwrap_or_default(), root)
            .map_err(|_| "failed to recompile program to resolve agent tool grants".to_string())?;
    let Some(agent_ir) = ir.agents.iter().find(|candidate| candidate.name == agent) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut specs = Vec::new();
    let mut entries = Vec::new();
    for tool in &agent_ir.tools {
        let resolved = crate::resolve_tool_grant(program_path, &ir, tool, package_lock_path)
            .map_err(|reason| format!("agent `{agent}` is granted `{tool}`: {reason}"))?;
        let (spec, entry) =
            tool_spec_and_entry(&resolved.tool_ir, resolved.source_path, resolved.package_id);
        specs.push(spec);
        entries.push(entry);
    }
    enforce_workflow_tool_invoke_governance(&entries)?;
    Ok((specs, entries))
}

fn enforce_workflow_tool_invoke_governance(entries: &[WorkflowToolEntry]) -> Result<(), String> {
    let resources = entries
        .iter()
        .filter(|entry| entry.package_id != crate::LOCAL_WORKFLOW_PACKAGE)
        .map(|entry| {
            (
                entry.name.as_str(),
                format!("invoke:{}/{}", entry.package_id, entry.name),
            )
        })
        .collect::<Vec<_>>();
    if resources.is_empty() {
        return Ok(());
    }
    match crate::ifc::VerifiedEnvelope::load_from_env() {
        crate::ifc::EnvelopeStatus::Ungoverned => Ok(()),
        crate::ifc::EnvelopeStatus::Rejected(message) => {
            Err(format!("governance envelope rejected: {message}"))
        }
        crate::ifc::EnvelopeStatus::Verified(verified) => {
            let missing = resources
                .into_iter()
                .filter(|(_, resource)| !verified.governs(resource))
                .map(|(name, resource)| format!("{name} ({resource})"))
                .collect::<Vec<_>>();
            if missing.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "cross-package workflow tool invoke door(s) not governed by the active envelope: {}",
                    missing.join(", ")
                ))
            }
        }
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

fn merge_grant_globs(slot: &mut Option<Vec<String>>, globs: Vec<String>) {
    match slot {
        None => *slot = Some(globs),
        Some(existing) if existing.is_empty() => {}
        Some(existing) if globs.is_empty() => existing.clear(),
        Some(existing) => {
            existing.extend(globs);
            existing.sort();
            existing.dedup();
        }
    }
}

fn globs_from_operation(operation: &Value) -> Result<Vec<String>, String> {
    let Some(globs) = operation.get("globs") else {
        return Ok(Vec::new());
    };
    let globs = globs
        .as_array()
        .ok_or_else(|| "access grant operation `globs` must be an array".to_owned())?;
    globs
        .iter()
        .map(|glob| {
            glob.as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| "access grant operation glob must be a non-empty string".to_owned())
        })
        .collect()
}

#[cfg(test)]
fn turn_file_access_from_input(input_json: &str) -> Result<TurnFileAccess, String> {
    Ok(turn_tool_access_from_input(input_json)?.file)
}

fn turn_tool_access_from_input(input_json: &str) -> Result<TurnToolAccess, String> {
    let input = serde_json::from_str::<Value>(input_json)
        .map_err(|error| format!("owned turn input is not valid JSON: {error}"))?;
    let Some(grants) = input.get("access_grants").and_then(Value::as_array) else {
        return Ok(TurnToolAccess::deny_all());
    };
    if grants.is_empty() {
        return Ok(TurnToolAccess::deny_all());
    }
    let mut store_names = Vec::<String>::new();
    let mut read_globs = None;
    let mut write_globs = None;
    let mut command_run = false;
    let mut tracker = TurnTrackerAccess::deny_all();
    for (grant_index, grant) in grants.iter().enumerate() {
        let resource = grant
            .get("resource")
            .and_then(Value::as_str)
            .filter(|resource| !resource.is_empty())
            .ok_or_else(|| format!("access_grants[{grant_index}] is missing `resource`"))?;
        let operations = grant
            .get("operations")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("access_grants[{grant_index}].operations must be an array"))?;
        let mut has_file_operation = false;
        for operation in operations {
            let operation_name = operation
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let globs = globs_from_operation(operation)?;
            match operation_name {
                "read" | "import" if resource != TRACKER_RESOURCE => {
                    has_file_operation = true;
                    merge_grant_globs(&mut read_globs, globs)
                }
                "write" | "export" if resource != TRACKER_RESOURCE => {
                    has_file_operation = true;
                    merge_grant_globs(&mut write_globs, globs)
                }
                "run" if resource == "command" => command_run = true,
                "file" | "add" if resource == TRACKER_RESOURCE => tracker.file = true,
                "claim" if resource == TRACKER_RESOURCE => tracker.claim = true,
                "finish" | "complete" | "close" if resource == TRACKER_RESOURCE => {
                    tracker.finish = true
                }
                "release" | "reopen" if resource == TRACKER_RESOURCE => tracker.release = true,
                "update" if resource == TRACKER_RESOURCE => tracker.grant_update(),
                "write" if resource == TRACKER_RESOURCE => tracker.grant_write(),
                _ => {}
            }
        }
        if has_file_operation && !store_names.iter().any(|existing| existing == resource) {
            store_names.push(resource.to_owned());
        }
    }
    let store_name = match store_names.as_slice() {
        [only] => only.clone(),
        [] => "turn_access".to_owned(),
        _ => "turn_access".to_owned(),
    };
    Ok(TurnToolAccess {
        file: TurnFileAccess {
            store_name,
            read_globs,
            write_globs,
        },
        file_resources: store_names,
        command_run,
        tracker,
    })
}

fn enforce_turn_access_governance(access: &TurnToolAccess) -> Result<(), String> {
    match crate::ifc::VerifiedEnvelope::load_from_env() {
        crate::ifc::EnvelopeStatus::Ungoverned => Ok(()),
        crate::ifc::EnvelopeStatus::Rejected(message) => {
            Err(format!("governance envelope rejected: {message}"))
        }
        crate::ifc::EnvelopeStatus::Verified(verified) => {
            let mut resources = access.file_resources.to_vec();
            if access.command_run {
                resources.push("command".to_owned());
            }
            if access.tracker.mutates() {
                resources.push(TRACKER_RESOURCE.to_owned());
            }
            let missing = resources
                .into_iter()
                .filter(|resource| !verified.governs(resource))
                .collect::<Vec<_>>();
            if missing.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "turn access grants resource(s) not governed by the active envelope: {}",
                    missing.join(", ")
                ))
            }
        }
    }
}

fn registered_profile_policy_from_store(
    store_path: &Path,
    profile: Option<&str>,
) -> StoreResult<Option<RegisteredProfilePolicy>> {
    let Some(profile) = profile else {
        return Ok(None);
    };
    SqliteStore::open(store_path)?.registered_profile_policy(profile)
}

fn required_capabilities_from_json(
    required_capabilities_json: &str,
) -> Result<Vec<String>, String> {
    let value = serde_json::from_str::<Value>(required_capabilities_json)
        .map_err(|error| format!("effect required_capabilities is not valid JSON: {error}"))?;
    let Some(items) = value.as_array() else {
        return Err("effect required_capabilities must be an array".to_owned());
    };
    let mut capabilities = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let Some(capability) = item.as_str().filter(|capability| !capability.is_empty()) else {
            return Err(format!(
                "effect required_capabilities[{index}] must be a non-empty string"
            ));
        };
        capabilities.push(capability.to_owned());
    }
    capabilities.sort();
    capabilities.dedup();
    Ok(capabilities)
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
    kernel: &mut RuntimeKernel<SqliteStore>,
    instance_id: &str,
    effect_id: &str,
    agent: &str,
    profile: Option<&str>,
    required_capabilities_json: &str,
    input_json: &str,
    store_path: &Path,
    max_child_iterations: usize,
    work_unit_root: Option<&str>,
    program_path: Option<&Path>,
    root: Option<&str>,
    package_lock_path: Option<&Path>,
    provider_ctx: crate::SubworkflowProviderContext,
) -> StoreResult<StoredEvent> {
    // Re-entrant workspace lease (DR-0025, amends slice 2). The lease holder is
    // the *root of the unit of work*, not the turn: a turn nested inside a
    // synchronous sub-workflow invocation (`work_unit_root` set) shares the
    // root's lease rather than contending with the parent that holds it, and
    // only the root releases. A top-level turn (`work_unit_root` None) is its own
    // root and holds the lease under its own instance id.
    let is_work_unit_root = work_unit_root.is_none();
    let work_unit = work_unit_root.unwrap_or(instance_id);
    // Resolve the model client before taking the workspace lease, so a config
    // error never leaks a held lease.
    let model_config = resolve_harness_model_config().map_err(StoreError::Conflict)?;
    // Discover `@tool` sub-workflows (DR-0025) up front: a non-convergent tool
    // fails the turn at setup, before the lease, so it never leaks a lease. Two
    // sources: the agent's in-program `tools [...]` grant (the curation surface)
    // and the `WHIPPLESCRIPT_HARNESS_TOOLS` operator override, merged (the grant
    // wins on a name collision).
    let (mut workflow_tool_specs, mut workflow_tools) =
        load_agent_granted_tools(program_path, root, agent, package_lock_path)
            .map_err(StoreError::Conflict)?;
    let (env_specs, env_entries) = load_workflow_tools().map_err(StoreError::Conflict)?;
    for (spec, entry) in env_specs.into_iter().zip(env_entries) {
        if workflow_tools
            .iter()
            .any(|existing| existing.name == entry.name)
        {
            continue;
        }
        workflow_tool_specs.push(spec);
        workflow_tools.push(entry);
    }
    let workspace = owned_workspace_root();
    let turn_tool_access = turn_tool_access_from_input(input_json).map_err(StoreError::Conflict)?;
    enforce_turn_access_governance(&turn_tool_access).map_err(StoreError::Conflict)?;
    let registered_profile_policy = registered_profile_policy_from_store(store_path, profile)?;
    let mut profile_policy = HarnessProfilePolicy::for_profile_with_registry(
        profile,
        registered_profile_policy.as_ref(),
    );
    let required_capabilities = required_capabilities_from_json(required_capabilities_json)
        .map_err(StoreError::Conflict)?;
    if let Some(required_policy) =
        HarnessProfilePolicy::from_required_capabilities(&required_capabilities)
    {
        profile_policy = profile_policy.intersect(&required_policy);
    }
    let mut executor = FileToolExecutor::new(&workspace)
        .with_turn_tool_access(turn_tool_access.clone())
        .with_resolved_profile_policy(profile_policy.clone());
    let mut tools = file_tool_specs_for_turn(&profile_policy, &turn_tool_access);
    // Tracker tools (slice 4): offered only when a tracker queue is configured.
    if let Some(queue) = std::env::var("WHIPPLESCRIPT_HARNESS_TRACKER")
        .ok()
        .filter(|value| !value.is_empty())
    {
        executor = executor.with_tracker(queue, instance_id);
        tools.extend(tracker_tool_specs_for_turn(
            &profile_policy,
            &turn_tool_access,
        ));
    }
    // Sub-workflow tools (DR-0025): curated, convergence-checked workflows the
    // model may invoke synchronously as typed tools.
    if !workflow_tools.is_empty() {
        executor = executor.with_workflow_tools(
            workflow_tools,
            store_path,
            max_child_iterations,
            work_unit,
            provider_ctx,
        );
        tools.extend(workflow_tool_specs_for_policy(
            &profile_policy,
            workflow_tool_specs,
        ));
    }
    // The registered-skills catalogue (context-assembly Phase 2): discover-all, so
    // every registered skill's name/description/location goes in and the model
    // reads a body on demand. A store read failure degrades to no catalogue.
    let skill_catalogue: Vec<SkillCatalogueEntry> = kernel
        .store()
        .list_skills()
        .unwrap_or_default()
        .into_iter()
        .map(|skill| SkillCatalogueEntry {
            name: skill.name,
            description: skill.description,
            location: skill.source_path,
        })
        .collect();
    // Skill activation (Decision 3): resolve each catalogue location to its
    // registered content-addressed body, so a `read` of that location returns the
    // exact registered bytes through the registry (not the filesystem — the read
    // then works identically on native and the durable object).
    let skill_bodies: std::collections::HashMap<String, String> = skill_catalogue
        .iter()
        .filter_map(|entry| {
            kernel
                .store()
                .skill_body(&entry.location)
                .ok()
                .flatten()
                .map(|body| (entry.location.clone(), body))
        })
        .collect();
    executor = executor.with_skill_bodies(skill_bodies);
    // Project instructions (AGENTS.md / CLAUDE.md) rooted at the workspace, plus an
    // optional env-configured global directory (context-assembly Phase 3).
    let global_context_dir =
        std::env::var_os("WHIPPLESCRIPT_GLOBAL_CONTEXT_DIR").map(PathBuf::from);
    let project_instructions = crate::project_context::discover_project_instructions(
        &workspace,
        global_context_dir.as_deref(),
    );
    // Assemble the system prompt from provenance-tagged bundles (mirror pi):
    // persona, tool snippets, guidelines, project context, available skills, date,
    // cwd. The host supplies date/cwd + the skill catalogue + project instructions;
    // the kernel assembler renders them in canonical order (context-assembly
    // Phase 1). Per-bundle provenance (`assembled.bundles`) is recorded as
    // `context.bundle` evidence by `run_brokered_agent_turn` (Decision 5).
    let assembled = assemble(owned_context_bundles(
        &tools,
        &owned_context_date(),
        &workspace.display().to_string(),
        &skill_catalogue,
        &project_instructions,
    ));
    let input = BrokeredTurnInput {
        system: assembled.system_prompt,
        user: input_json.to_string(),
        tools,
        max_steps: owned_max_steps(),
        // The runner populates resume_from from any persisted transcript on
        // crash recovery (slice 6); a fresh turn starts empty.
        resume_from: Vec::new(),
        // Per-bundle provenance for the assembled prompt; the runner records one
        // context.bundle evidence row each before the turn (Decision 5).
        context_bundles: assembled.bundles,
    };
    let ctx = BrokeredTurnContext {
        instance_id,
        effect_id,
        agent,
        profile,
    };

    // Slice-2 envelope: hold a durable workspace lease for the unit of work so
    // concurrent *root* owned turns coordinate on a shared workspace. A contended
    // workspace blocks (recoverable) rather than racing; a later worker pass runs
    // it once free. The lease is keyed on the work-unit root (DR-0025), so a
    // sub-workflow turn re-acquires the lease its own root already holds (`Held`,
    // idempotent) instead of self-deadlocking.
    let resource = "owned.workspace";
    let key = workspace.display().to_string();
    let mut coordination = CoordinationStore::open(crate::coordination_store_path())?;
    match coordination.try_acquire(resource, &key, 1, OWNED_LEASE_TTL_SECONDS, work_unit)? {
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
                // Stable cache key for this turn-thread (Decision 7): the effect id,
                // constant across the turn's model steps.
                Some(effect_id.to_owned()),
            );
            kernel.run_brokered_agent_turn(&ctx, &client, &executor, &input)
        }
        None => {
            let client = FixtureModelClient::from_env();
            kernel.run_brokered_agent_turn(&ctx, &client, &executor, &input)
        }
    };

    // Release the workspace lease on every terminal (success or failure), mirroring
    // release_holder_resources_on_terminal for effect-held coordination. Only the
    // work-unit root releases: a nested sub-workflow turn shares the root's lease
    // and must not drop it out from under the still-running parent (DR-0025).
    if is_work_unit_root {
        if let Ok(mut coordination) = CoordinationStore::open(crate::coordination_store_path()) {
            let _ = coordination.release(resource, &key, work_unit);
        }
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

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    fn owned_context_prompt_mirrors_pi_shape_and_keeps_authority_contract() {
        let tools = vec![ToolSpec {
            name: "read".into(),
            description: "Read a file from the workspace.".into(),
            input_schema: json!({}),
        }];
        let assembled = assemble(owned_context_bundles(
            &tools,
            "2026-07-04",
            "/repo",
            &[],
            &[],
        ));
        let prompt = assembled.system_prompt;

        // Persona + guidelines carry the turn-scoped authority + termination contract.
        assert!(prompt.contains("authority granted for this turn"));
        assert!(prompt.contains("make no further tool calls"));
        // pi-shape: the tool list is enumerated in prose (one line per tool).
        assert!(prompt.contains("Available tools:"));
        assert!(prompt.contains("- read: Read a file from the workspace."));
        // Date + cwd bundles are present.
        assert!(prompt.contains("Current date: 2026-07-04"));
        assert!(prompt.contains("Current working directory: /repo"));
        // Canonical order: persona/tools/guidelines before date before cwd.
        let persona_at = prompt
            .find("expert coding assistant")
            .expect("persona marker present");
        let tools_at = prompt
            .find("Available tools:")
            .expect("tools marker present");
        let date_at = prompt.find("Current date:").expect("date marker present");
        let cwd_at = prompt
            .find("Current working directory:")
            .expect("cwd marker present");
        assert!(persona_at < tools_at && tools_at < date_at && date_at < cwd_at);
        // One provenance row per included bundle (persona, tools, guidelines, date, cwd).
        assert_eq!(assembled.bundles.len(), 5);
    }

    #[test]
    fn owned_context_prompt_omits_tool_list_when_no_tools_offered() {
        let assembled = assemble(owned_context_bundles(&[], "2026-07-04", "/repo", &[], &[]));
        assert!(!assembled.system_prompt.contains("Available tools:"));
        // persona, guidelines, date, cwd -- no tools bundle.
        assert_eq!(assembled.bundles.len(), 4);
    }

    #[test]
    fn available_skills_catalogue_renders_only_with_a_read_tool() {
        let read = vec![ToolSpec {
            name: "read".into(),
            description: "Read a file.".into(),
            input_schema: json!({}),
        }];
        let skills = vec![SkillCatalogueEntry {
            name: "triage".into(),
            description: "Triage the inbox.".into(),
            location: ".whipplescript/skills/triage/SKILL.md".into(),
        }];

        // With a read tool present, the catalogue renders name/description/location.
        let with_read = assemble(owned_context_bundles(
            &read,
            "2026-07-04",
            "/repo",
            &skills,
            &[],
        ));
        assert!(with_read.system_prompt.contains("<available_skills>"));
        assert!(with_read.system_prompt.contains(
            "<skill name=\"triage\" location=\".whipplescript/skills/triage/SKILL.md\">"
        ));
        assert!(with_read.system_prompt.contains("Triage the inbox."));
        assert!(with_read
            .bundles
            .iter()
            .any(|bundle| bundle.kind == BundleKind::AvailableSkills));

        // Without a read-class tool the model can't fetch a body, so no catalogue.
        let no_read = assemble(owned_context_bundles(
            &[],
            "2026-07-04",
            "/repo",
            &skills,
            &[],
        ));
        assert!(!no_read.system_prompt.contains("<available_skills>"));
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
    fn read_of_a_skill_location_resolves_the_registry_body_not_the_filesystem() {
        let root = temp_root();
        let mut bodies = std::collections::HashMap::new();
        bodies.insert(
            "skills/demo/SKILL.md".to_string(),
            "# Demo\nregistry body bytes\n".to_string(),
        );
        let exec = FileToolExecutor::new(&root).with_skill_bodies(bodies);
        // The location is not a file under root, yet the read succeeds from the
        // registry — bypassing the filesystem and the file-glob policy (Decision 3).
        let r = exec.execute(&call(TOOL_READ, json!({ "path": "skills/demo/SKILL.md" })));
        assert_eq!(r.status, ToolStatus::Ok);
        assert!(r.content.contains("registry body bytes"));
        // A non-skill path still resolves against the filesystem (missing here).
        let miss = exec.execute(&call(TOOL_READ, json!({ "path": "nope.txt" })));
        assert_eq!(miss.status, ToolStatus::Error);
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
    fn turn_file_access_denies_file_tools_without_grants() {
        let root = temp_root();
        std::fs::write(root.join("note.txt"), "secret").expect("seed");
        let access = turn_file_access_from_input(r#"{"prompt":"work"}"#).expect("parse input");
        let exec = FileToolExecutor::new(&root).with_turn_file_access(access);

        let blocked = exec.execute(&call(TOOL_READ, json!({ "path": "note.txt" })));

        assert_eq!(blocked.status, ToolStatus::Error);
        assert!(blocked.content.contains("not granted"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn turn_file_access_applies_read_and_write_globs() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("src")).expect("src dir");
        std::fs::write(root.join("src/in.txt"), "ok").expect("seed");
        let input = json!({
            "access_grants": [
                {
                    "resource": "project_files",
                    "operations": [
                        {"operation": "read", "globs": ["src/**"]},
                        {"operation": "write", "globs": ["out/**"]}
                    ]
                }
            ]
        })
        .to_string();
        let access = turn_file_access_from_input(&input).expect("parse grants");
        let exec = FileToolExecutor::new(&root).with_turn_file_access(access);

        let read_allowed = exec.execute(&call(TOOL_READ, json!({ "path": "src/in.txt" })));
        let read_blocked = exec.execute(&call(TOOL_READ, json!({ "path": "secret.txt" })));
        let write_allowed = exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "out/new.txt", "content": "ok" }),
        ));
        let write_blocked = exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "src/new.txt", "content": "no" }),
        ));

        assert_eq!(read_allowed.status, ToolStatus::Ok);
        assert_eq!(read_blocked.status, ToolStatus::Error);
        assert_eq!(write_allowed.status, ToolStatus::Ok);
        assert_eq!(write_blocked.status, ToolStatus::Error);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn turn_tool_access_tracks_file_resources_for_governance() {
        let input = json!({
            "access_grants": [
                {
                    "resource": "project_files",
                    "operations": [
                        {"operation": "read", "globs": ["src/**"]}
                    ]
                },
                {
                    "resource": "command",
                    "operations": [
                        {"operation": "run"}
                    ]
                },
                {
                    "resource": "docs",
                    "operations": [
                        {"operation": "write", "globs": ["docs/**"]}
                    ]
                }
            ]
        })
        .to_string();

        let access = turn_tool_access_from_input(&input).expect("parse grants");

        assert_eq!(
            access.file_resources,
            vec!["project_files".to_owned(), "docs".to_owned()]
        );
        assert!(access.command_run);
    }

    #[test]
    fn tracker_write_grants_filter_model_facing_tracker_tools() {
        let policy = HarnessProfilePolicy::for_profile(Some("repo-writer"));
        let no_tracker = turn_tool_access_from_input(r#"{"prompt":"work"}"#)
            .expect("missing grants deny tracker writes");
        let no_tracker_names = tracker_tool_specs_for_turn(&policy, &no_tracker)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(no_tracker_names, vec![TOOL_LIST_TODOS.to_owned()]);

        let file_only = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "tracker",
                        "operations": [
                            {"operation": "file"}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("tracker file grant parses");
        let file_names = tracker_tool_specs_for_turn(&policy, &file_only)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(
            file_names,
            vec![TOOL_LIST_TODOS.to_owned(), TOOL_ADD_TODO.to_owned()]
        );

        let update = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "tracker",
                        "operations": [
                            {"operation": "finish"}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("tracker update grant parses");
        let update_names = tracker_tool_specs_for_turn(&policy, &update)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(
            update_names,
            vec![TOOL_LIST_TODOS.to_owned(), TOOL_UPDATE_TODO.to_owned()]
        );

        let reader_policy = HarnessProfilePolicy::for_profile(Some("repo-reader"));
        let reader_names = tracker_tool_specs_for_turn(&reader_policy, &file_only)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(reader_names, vec![TOOL_LIST_TODOS.to_owned()]);
    }

    #[test]
    fn tracker_mutations_require_turn_grants_and_status_specific_update_grants() {
        let root = temp_root();
        let no_tracker = turn_tool_access_from_input(r#"{"prompt":"work"}"#)
            .expect("missing grants deny tracker writes");
        let exec = FileToolExecutor::new(&root)
            .with_tracker("queue", "instance")
            .with_turn_tool_access(no_tracker)
            .with_profile_policy(Some("repo-writer"));

        let add = exec.execute(&call(TOOL_ADD_TODO, json!({ "content": "do a thing" })));
        assert_eq!(add.status, ToolStatus::Error);
        assert!(add.content.contains("tracker file is not granted"));

        let claim_only = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "tracker",
                        "operations": [
                            {"operation": "claim"}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("claim grant parses");
        let exec = FileToolExecutor::new(&root)
            .with_tracker("queue", "instance")
            .with_turn_tool_access(claim_only)
            .with_profile_policy(Some("repo-writer"));
        let finish = exec.execute(&call(
            TOOL_UPDATE_TODO,
            json!({ "id": "item-1", "status": "completed" }),
        ));
        assert_eq!(finish.status, ToolStatus::Error);
        assert!(finish.content.contains("tracker update is not granted"));
        assert!(finish.content.contains("finish"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn turn_access_governance_requires_envelope_to_cover_file_resources() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let previous_envelope = std::env::var_os("WHIPPLESCRIPT_IFC_ENVELOPE");
        let root = temp_root();
        let envelope_path = root.join("env.policy");

        std::fs::write(
            &envelope_path,
            "grant file_store project_files -> file:/srv/project public\n",
        )
        .expect("write envelope");
        std::env::set_var("WHIPPLESCRIPT_IFC_ENVELOPE", &envelope_path);

        let governed = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "project_files",
                        "operations": [
                            {"operation": "read", "globs": ["src/**"]}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("governed grant parses");
        enforce_turn_access_governance(&governed).expect("resource is governed");

        let ungoverned = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "secret_files",
                        "operations": [
                            {"operation": "read", "globs": ["secrets/**"]}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("ungoverned grant parses");
        let error = enforce_turn_access_governance(&ungoverned)
            .expect_err("ungoverned resource must fail closed");
        assert!(error.contains("secret_files"));
        assert!(error.contains("not governed"));

        let command = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "command",
                        "operations": [
                            {"operation": "run"}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("command grant parses");
        let error = enforce_turn_access_governance(&command)
            .expect_err("ungoverned command must fail closed");
        assert!(error.contains("command"));

        std::fs::write(&envelope_path, "grant command command -> command public\n")
            .expect("write command envelope");
        enforce_turn_access_governance(&command).expect("command resource is governed");

        let tracker = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "tracker",
                        "operations": [
                            {"operation": "file"}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("tracker grant parses");
        let error = enforce_turn_access_governance(&tracker)
            .expect_err("ungoverned tracker must fail closed");
        assert!(error.contains("tracker"));

        std::fs::write(&envelope_path, "grant tracker tracker -> tracker public\n")
            .expect("write tracker envelope");
        enforce_turn_access_governance(&tracker).expect("tracker resource is governed");

        match previous_envelope {
            Some(value) => std::env::set_var("WHIPPLESCRIPT_IFC_ENVELOPE", value),
            None => std::env::remove_var("WHIPPLESCRIPT_IFC_ENVELOPE"),
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn package_workflow_tool_invoke_requires_governed_door() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let previous_envelope = std::env::var_os("WHIPPLESCRIPT_IFC_ENVELOPE");
        let root = temp_root();
        let envelope_path = root.join("env.policy");

        let entry = WorkflowToolEntry {
            name: "LeakyTool".to_owned(),
            path: root.join("tool.whip"),
            root: "LeakyTool".to_owned(),
            package_id: "package-leaky".to_owned(),
        };
        let local_entry = WorkflowToolEntry {
            name: "LocalTool".to_owned(),
            path: root.join("local.whip"),
            root: "LocalTool".to_owned(),
            package_id: crate::LOCAL_WORKFLOW_PACKAGE.to_owned(),
        };

        enforce_workflow_tool_invoke_governance(std::slice::from_ref(&local_entry))
            .expect("same-bundle workflow tools do not cross a package boundary");

        std::fs::write(
            &envelope_path,
            "grant file_store project_files -> file:/srv/project public\n",
        )
        .expect("write envelope");
        std::env::set_var("WHIPPLESCRIPT_IFC_ENVELOPE", &envelope_path);

        let error = enforce_workflow_tool_invoke_governance(std::slice::from_ref(&entry))
            .expect_err("cross-package tool invoke must be governed");
        assert!(error.contains("LeakyTool"));
        assert!(error.contains("invoke:package-leaky/LeakyTool"));

        std::fs::write(
            &envelope_path,
            "grant invoke LeakyTool -> invoke:package-leaky/LeakyTool public\n",
        )
        .expect("write invoke envelope");
        enforce_workflow_tool_invoke_governance(std::slice::from_ref(&entry))
            .expect("cross-package invoke door is governed");

        match previous_envelope {
            Some(value) => std::env::set_var("WHIPPLESCRIPT_IFC_ENVELOPE", value),
            None => std::env::remove_var("WHIPPLESCRIPT_IFC_ENVELOPE"),
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn turn_file_access_edit_requires_read_and_write() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("src")).expect("src dir");
        std::fs::write(root.join("src/in.txt"), "old").expect("seed");

        let write_only_input = json!({
            "access_grants": [
                {
                    "resource": "project_files",
                    "operations": [
                        {"operation": "write", "globs": ["src/**"]}
                    ]
                }
            ]
        })
        .to_string();
        let write_only_access =
            turn_file_access_from_input(&write_only_input).expect("parse write grants");
        let write_only_exec = FileToolExecutor::new(&root).with_turn_file_access(write_only_access);
        let missing_read = write_only_exec.execute(&call(
            TOOL_EDIT,
            json!({ "path": "src/in.txt", "edits": [{ "oldText": "old", "newText": "new" }] }),
        ));

        assert_eq!(missing_read.status, ToolStatus::Error);
        assert!(missing_read.content.contains("read is not granted"));
        assert_eq!(
            std::fs::read_to_string(root.join("src/in.txt")).expect("read src/in.txt"),
            "old"
        );

        let read_only_input = json!({
            "access_grants": [
                {
                    "resource": "project_files",
                    "operations": [
                        {"operation": "read", "globs": ["src/**"]}
                    ]
                }
            ]
        })
        .to_string();
        let read_only_access =
            turn_file_access_from_input(&read_only_input).expect("parse read grants");
        let read_only_exec = FileToolExecutor::new(&root).with_turn_file_access(read_only_access);
        let missing_write = read_only_exec.execute(&call(
            TOOL_EDIT,
            json!({ "path": "src/in.txt", "edits": [{ "oldText": "old", "newText": "new" }] }),
        ));

        assert_eq!(missing_write.status, ToolStatus::Error);
        assert!(missing_write.content.contains("write is not granted"));
        assert_eq!(
            std::fs::read_to_string(root.join("src/in.txt")).expect("read src/in.txt"),
            "old"
        );

        let read_write_input = json!({
            "access_grants": [
                {
                    "resource": "project_files",
                    "operations": [
                        {"operation": "read", "globs": ["src/**"]},
                        {"operation": "write", "globs": ["src/**"]}
                    ]
                }
            ]
        })
        .to_string();
        let read_write_access =
            turn_file_access_from_input(&read_write_input).expect("parse read/write grants");
        let read_write_exec = FileToolExecutor::new(&root).with_turn_file_access(read_write_access);
        let edited = read_write_exec.execute(&call(
            TOOL_EDIT,
            json!({ "path": "src/in.txt", "edits": [{ "oldText": "old", "newText": "new" }] }),
        ));

        assert_eq!(edited.status, ToolStatus::Ok);
        assert_eq!(
            std::fs::read_to_string(root.join("src/in.txt")).expect("read src/in.txt"),
            "new"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn profile_policy_intersects_file_and_bash_tools() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("src")).expect("src dir");
        std::fs::write(root.join("src/in.txt"), "old").expect("seed");
        let input = json!({
            "access_grants": [
                {
                    "resource": "project_files",
                    "operations": [
                        {"operation": "read", "globs": ["src/**"]},
                        {"operation": "write", "globs": ["src/**"]}
                    ]
                }
            ]
        })
        .to_string();
        let access = turn_file_access_from_input(&input).expect("parse grants");
        let exec = FileToolExecutor::new(&root)
            .with_turn_file_access(access)
            .with_profile_policy(Some("repo-reader"))
            .with_bash_allow(vec!["echo".into()]);

        let read = exec.execute(&call(TOOL_READ, json!({ "path": "src/in.txt" })));
        let write = exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "src/out.txt", "content": "new" }),
        ));
        let edit = exec.execute(&call(
            TOOL_EDIT,
            json!({ "path": "src/in.txt", "edits": [{ "oldText": "old", "newText": "new" }] }),
        ));
        let bash = exec.execute(&call(TOOL_BASH, json!({ "command": "echo hello" })));

        assert_eq!(read.status, ToolStatus::Ok);
        assert_eq!(write.status, ToolStatus::Error);
        assert!(write.content.contains("profile `repo-reader`"));
        assert_eq!(edit.status, ToolStatus::Error);
        assert!(edit.content.contains("profile `repo-reader`"));
        assert_eq!(bash.status, ToolStatus::Error);
        assert!(bash.content.contains("profile `repo-reader`"));
        assert_eq!(
            std::fs::read_to_string(root.join("src/in.txt")).expect("read src/in.txt"),
            "old"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_repo_profile_blocks_file_tools_even_with_grants() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("src")).expect("src dir");
        std::fs::write(root.join("src/in.txt"), "old").expect("seed");
        let input = json!({
            "access_grants": [
                {
                    "resource": "project_files",
                    "operations": [
                        {"operation": "read", "globs": ["src/**"]}
                    ]
                }
            ]
        })
        .to_string();
        let access = turn_file_access_from_input(&input).expect("parse grants");
        let exec = FileToolExecutor::new(&root)
            .with_turn_file_access(access)
            .with_profile_policy(Some("no-repo"));

        let read = exec.execute(&call(TOOL_READ, json!({ "path": "src/in.txt" })));

        assert_eq!(read.status, ToolStatus::Error);
        assert!(read.content.contains("profile `no-repo`"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn profile_policy_filters_model_facing_file_tools() {
        let names = |profile| {
            file_tool_specs_for_profile(profile)
                .into_iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>()
        };

        let reader = names(Some("repo-reader"));
        assert!(reader.contains(&TOOL_READ.to_owned()));
        assert!(reader.contains(&TOOL_GREP.to_owned()));
        assert!(reader.contains(&TOOL_FIND.to_owned()));
        assert!(reader.contains(&TOOL_LS.to_owned()));
        assert!(!reader.contains(&TOOL_WRITE.to_owned()));
        assert!(!reader.contains(&TOOL_EDIT.to_owned()));
        assert!(!reader.contains(&TOOL_BASH.to_owned()));

        let writer = names(Some("repo-writer"));
        assert!(writer.contains(&TOOL_WRITE.to_owned()));
        assert!(writer.contains(&TOOL_EDIT.to_owned()));
        assert!(writer.contains(&TOOL_BASH.to_owned()));

        assert!(names(Some("no-repo")).is_empty());
    }

    #[test]
    fn command_run_turn_grant_filters_model_facing_bash_tool() {
        let policy = HarnessProfilePolicy::for_profile(Some("repo-writer"));
        let without_command = turn_tool_access_from_input(r#"{"prompt":"work"}"#)
            .expect("missing grants deny command");
        let with_command = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "command",
                        "operations": [
                            {"operation": "run"}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("command grant parses");

        let without_names = file_tool_specs_for_turn(&policy, &without_command)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        let with_names = file_tool_specs_for_turn(&policy, &with_command)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();

        assert!(!without_names.contains(&TOOL_BASH.to_owned()));
        assert!(with_names.contains(&TOOL_BASH.to_owned()));
    }

    #[test]
    fn required_capabilities_intersect_owned_harness_tool_policy() {
        let base = HarnessProfilePolicy::for_profile(Some("repo-writer"));
        let access = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "project_files",
                        "operations": [
                            {"operation": "read", "globs": ["src/**"]},
                            {"operation": "write", "globs": ["src/**"]}
                        ]
                    },
                    {
                        "resource": "command",
                        "operations": [
                            {"operation": "run"}
                        ]
                    },
                    {
                        "resource": "tracker",
                        "operations": [
                            {"operation": "write"}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("turn grants parse");

        let required = |capabilities: &[&str]| {
            let capabilities = capabilities
                .iter()
                .map(|capability| (*capability).to_owned())
                .collect::<Vec<_>>();
            HarnessProfilePolicy::from_required_capabilities(&capabilities)
        };
        let file_names = |policy: &HarnessProfilePolicy| {
            file_tool_specs_for_turn(policy, &access)
                .into_iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>()
        };
        let tracker_names = |policy: &HarnessProfilePolicy| {
            tracker_tool_specs_for_turn(policy, &access)
                .into_iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>()
        };
        let workflow_names = |policy: &HarnessProfilePolicy| {
            workflow_tool_specs_for_policy(
                policy,
                vec![ToolSpec {
                    name: "EchoTool".to_owned(),
                    description: "Echo test tool".to_owned(),
                    input_schema: json!({"type": "object"}),
                }],
            )
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>()
        };

        assert!(required(&["agent.tell"]).is_none());

        let read_only = base.intersect(&required(&["repo.read"]).expect("repo.read policy"));
        let read_names = file_names(&read_only);
        assert!(read_names.contains(&TOOL_READ.to_owned()));
        assert!(read_names.contains(&TOOL_GREP.to_owned()));
        assert!(!read_names.contains(&TOOL_WRITE.to_owned()));
        assert!(!read_names.contains(&TOOL_BASH.to_owned()));
        assert_eq!(tracker_names(&read_only), vec![TOOL_LIST_TODOS.to_owned()]);
        assert!(workflow_names(&read_only).is_empty());
        let root = temp_root();
        std::fs::create_dir_all(root.join("src")).expect("src dir");
        std::fs::write(root.join("src/in.txt"), "old").expect("seed");
        let exec = FileToolExecutor::new(&root)
            .with_turn_tool_access(access.clone())
            .with_resolved_profile_policy(read_only.clone());
        let read = exec.execute(&call(TOOL_READ, json!({ "path": "src/in.txt" })));
        let write = exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "src/out.txt", "content": "new" }),
        ));
        assert_eq!(read.status, ToolStatus::Ok);
        assert_eq!(write.status, ToolStatus::Error);
        assert!(write.content.contains("profile `repo-writer`"));
        std::fs::remove_dir_all(&root).ok();

        let command_only = base.intersect(&required(&["command.run"]).expect("command.run policy"));
        let command_names = file_names(&command_only);
        assert_eq!(command_names, vec![TOOL_BASH.to_owned()]);
        assert_eq!(
            tracker_names(&command_only),
            vec![TOOL_LIST_TODOS.to_owned()]
        );
        assert!(workflow_names(&command_only).is_empty());

        let tracker_finish =
            base.intersect(&required(&["tracker.finish"]).expect("tracker.finish policy"));
        assert!(file_names(&tracker_finish).is_empty());
        assert_eq!(
            tracker_names(&tracker_finish),
            vec![TOOL_LIST_TODOS.to_owned(), TOOL_UPDATE_TODO.to_owned()]
        );
        assert!(workflow_names(&tracker_finish).is_empty());

        let workflow_only =
            base.intersect(&required(&["workflow.invoke"]).expect("workflow.invoke policy"));
        assert!(file_names(&workflow_only).is_empty());
        assert_eq!(
            tracker_names(&workflow_only),
            vec![TOOL_LIST_TODOS.to_owned()]
        );
        assert_eq!(workflow_names(&workflow_only), vec!["EchoTool".to_owned()]);
    }

    #[test]
    fn required_capabilities_json_must_be_a_string_array() {
        assert_eq!(
            required_capabilities_from_json(r#"["agent.tell","repo.read","repo.read"]"#)
                .expect("valid required capabilities"),
            vec!["agent.tell".to_owned(), "repo.read".to_owned()]
        );
        assert!(
            required_capabilities_from_json(r#"{"capability":"repo.read"}"#)
                .expect_err("non-array rejects")
                .contains("must be an array")
        );
        assert!(required_capabilities_from_json(r#"[1]"#)
            .expect_err("non-string rejects")
            .contains("non-empty string"));
    }

    #[test]
    fn turn_file_grants_filter_model_facing_file_tools() {
        let policy = HarnessProfilePolicy::for_profile(Some("repo-writer"));
        let names_for = |input: Value| {
            let access =
                turn_tool_access_from_input(&input.to_string()).expect("turn grants parse");
            file_tool_specs_for_turn(&policy, &access)
                .into_iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>()
        };

        let read_only = names_for(json!({
            "access_grants": [
                {
                    "resource": "project_files",
                    "operations": [
                        {"operation": "read", "globs": ["src/**"]}
                    ]
                }
            ]
        }));
        assert!(read_only.contains(&TOOL_READ.to_owned()));
        assert!(read_only.contains(&TOOL_GREP.to_owned()));
        assert!(read_only.contains(&TOOL_FIND.to_owned()));
        assert!(read_only.contains(&TOOL_LS.to_owned()));
        assert!(!read_only.contains(&TOOL_WRITE.to_owned()));
        assert!(!read_only.contains(&TOOL_EDIT.to_owned()));

        let write_only = names_for(json!({
            "access_grants": [
                {
                    "resource": "project_files",
                    "operations": [
                        {"operation": "write", "globs": ["src/**"]}
                    ]
                }
            ]
        }));
        assert!(!write_only.contains(&TOOL_READ.to_owned()));
        assert!(write_only.contains(&TOOL_WRITE.to_owned()));
        assert!(!write_only.contains(&TOOL_EDIT.to_owned()));

        let read_write = names_for(json!({
            "access_grants": [
                {
                    "resource": "project_files",
                    "operations": [
                        {"operation": "read", "globs": ["src/**"]},
                        {"operation": "write", "globs": ["src/**"]}
                    ]
                }
            ]
        }));
        assert!(read_write.contains(&TOOL_READ.to_owned()));
        assert!(read_write.contains(&TOOL_WRITE.to_owned()));
        assert!(read_write.contains(&TOOL_EDIT.to_owned()));
    }

    #[test]
    fn registered_custom_profile_policy_filters_model_facing_file_tools() {
        let registered = RegisteredProfilePolicy {
            enforcement_mode: "enforce".to_owned(),
            allowed_capabilities: vec!["repo.read".to_owned()],
        };
        let policy =
            HarnessProfilePolicy::for_profile_with_registry(Some("docs-reader"), Some(&registered));
        let names = file_tool_specs_for_policy(&policy)
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();

        assert!(names.contains(&TOOL_READ.to_owned()));
        assert!(names.contains(&TOOL_GREP.to_owned()));
        assert!(names.contains(&TOOL_FIND.to_owned()));
        assert!(names.contains(&TOOL_LS.to_owned()));
        assert!(!names.contains(&TOOL_WRITE.to_owned()));
        assert!(!names.contains(&TOOL_EDIT.to_owned()));
        assert!(!names.contains(&TOOL_BASH.to_owned()));
        assert!(workflow_tool_specs_for_policy(
            &policy,
            vec![ToolSpec {
                name: "EchoTool".to_owned(),
                description: "Echo test tool".to_owned(),
                input_schema: json!({"type": "object"}),
            }]
        )
        .is_empty());
    }

    #[test]
    fn registered_custom_profile_policy_filters_workflow_tools() {
        let workflow_tool = || ToolSpec {
            name: "EchoTool".to_owned(),
            description: "Echo test tool".to_owned(),
            input_schema: json!({"type": "object"}),
        };
        let registered_without_invoke = RegisteredProfilePolicy {
            enforcement_mode: "enforce".to_owned(),
            allowed_capabilities: vec!["repo.read".to_owned()],
        };
        let without_invoke = HarnessProfilePolicy::for_profile_with_registry(
            Some("docs-reader"),
            Some(&registered_without_invoke),
        );
        assert!(workflow_tool_specs_for_policy(&without_invoke, vec![workflow_tool()]).is_empty());

        let registered_with_invoke = RegisteredProfilePolicy {
            enforcement_mode: "enforce".to_owned(),
            allowed_capabilities: vec!["workflow.invoke".to_owned()],
        };
        let with_invoke = HarnessProfilePolicy::for_profile_with_registry(
            Some("tool-runner"),
            Some(&registered_with_invoke),
        );
        let names = workflow_tool_specs_for_policy(&with_invoke, vec![workflow_tool()])
            .into_iter()
            .map(|spec| spec.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["EchoTool".to_owned()]);
    }

    #[test]
    fn workflow_tool_dispatch_requires_profile_capability() {
        let root = temp_root();
        let mut exec = FileToolExecutor::new(&root).with_resolved_profile_policy(
            HarnessProfilePolicy::from_required_capabilities(&["repo.read".to_owned()])
                .expect("repo.read required policy"),
        );
        exec.workflow_tools.push(WorkflowToolEntry {
            name: "EchoTool".to_owned(),
            path: root.join("tool.whip"),
            root: "EchoTool".to_owned(),
            package_id: crate::LOCAL_WORKFLOW_PACKAGE.to_owned(),
        });

        let denied = exec.execute(&call("EchoTool", json!({})));
        assert_eq!(denied.status, ToolStatus::Error);
        assert!(denied
            .content
            .contains("workflow tool invoke is not permitted"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn registered_custom_profile_policy_intersects_file_and_bash_tools() {
        let root = temp_root();
        std::fs::create_dir_all(root.join("src")).expect("src dir");
        std::fs::write(root.join("src/in.txt"), "old").expect("seed");
        let input = json!({
            "access_grants": [
                {
                    "resource": "project_files",
                    "operations": [
                        {"operation": "read", "globs": ["src/**"]},
                        {"operation": "write", "globs": ["src/**"]}
                    ]
                }
            ]
        })
        .to_string();
        let access = turn_file_access_from_input(&input).expect("parse grants");
        let registered = RegisteredProfilePolicy {
            enforcement_mode: "enforce".to_owned(),
            allowed_capabilities: vec!["repo.read".to_owned()],
        };
        let policy =
            HarnessProfilePolicy::for_profile_with_registry(Some("docs-reader"), Some(&registered));
        let exec = FileToolExecutor::new(&root)
            .with_turn_file_access(access)
            .with_resolved_profile_policy(policy)
            .with_bash_allow(vec!["echo".into()]);

        let read = exec.execute(&call(TOOL_READ, json!({ "path": "src/in.txt" })));
        let write = exec.execute(&call(
            TOOL_WRITE,
            json!({ "path": "src/out.txt", "content": "new" }),
        ));
        let bash = exec.execute(&call(TOOL_BASH, json!({ "command": "echo hello" })));

        assert_eq!(read.status, ToolStatus::Ok);
        assert_eq!(write.status, ToolStatus::Error);
        assert!(write.content.contains("profile `docs-reader`"));
        assert_eq!(bash.status, ToolStatus::Error);
        assert!(bash.content.contains("profile `docs-reader`"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn registered_profile_policy_loads_from_store() {
        let root = temp_root();
        let store_path = root.join("profile-store.sqlite");
        let store = SqliteStore::open(&store_path).expect("store opens");
        store
            .register_profile(whipplescript_store::ProfileRegistration {
                profile_id: "profile_docs_reader",
                name: "docs-reader",
                description: "Read project docs.",
                enforcement_mode: "enforce",
                allowed_capabilities_json: r#"["repo.read"]"#,
                config_json: "{}",
            })
            .expect("profile registers");
        drop(store);

        let registered = registered_profile_policy_from_store(&store_path, Some("docs-reader"))
            .expect("profile lookup succeeds")
            .expect("profile exists");

        assert_eq!(registered.enforcement_mode, "enforce");
        assert_eq!(registered.allowed_capabilities, vec!["repo.read"]);
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
    fn bash_requires_command_run_turn_grant_when_turn_policy_is_installed() {
        let root = temp_root();
        let read_only = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "project_files",
                        "operations": [
                            {"operation": "read", "globs": ["src/**"]}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("read-only grant parses");
        let exec = FileToolExecutor::new(&root)
            .with_turn_tool_access(read_only)
            .with_profile_policy(Some("repo-writer"))
            .with_bash_allow(vec!["echo".into()]);

        let denied = exec.execute(&call(TOOL_BASH, json!({ "command": "echo hello" })));

        assert_eq!(denied.status, ToolStatus::Error);
        assert!(denied.content.contains("command { run }"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bash_runs_when_profile_turn_grant_and_allow_list_all_permit() {
        let root = temp_root();
        let command_only = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "command",
                        "operations": [
                            {"operation": "run"}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("command grant parses");
        let exec = FileToolExecutor::new(&root)
            .with_turn_tool_access(command_only)
            .with_profile_policy(Some("repo-writer"))
            .with_bash_allow(vec!["echo".into()]);

        let ok = exec.execute(&call(TOOL_BASH, json!({ "command": "echo hello" })));

        assert_eq!(ok.status, ToolStatus::Ok);
        assert!(ok.content.contains("hello"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bash_output_redirection_requires_turn_write_grant() {
        let root = temp_root();
        let command_only = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "command",
                        "operations": [
                            {"operation": "run"}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("command grant parses");
        let exec = FileToolExecutor::new(&root)
            .with_turn_tool_access(command_only)
            .with_profile_policy(Some("repo-writer"))
            .with_bash_allow(vec!["echo".into()]);

        let denied = exec.execute(&call(
            TOOL_BASH,
            json!({ "command": "echo hello > out.txt" }),
        ));

        assert_eq!(denied.status, ToolStatus::Error);
        assert!(denied.content.contains("out.txt"));
        assert!(denied.content.contains("file write is not granted"));
        assert!(!root.join("out.txt").exists());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bash_input_redirection_requires_turn_read_grant() {
        let root = temp_root();
        std::fs::write(root.join("input.txt"), "hello\n").expect("seed input");
        let command_only = turn_tool_access_from_input(
            &json!({
                "access_grants": [
                    {
                        "resource": "command",
                        "operations": [
                            {"operation": "run"}
                        ]
                    }
                ]
            })
            .to_string(),
        )
        .expect("command grant parses");
        let exec = FileToolExecutor::new(&root)
            .with_turn_tool_access(command_only)
            .with_profile_policy(Some("repo-writer"))
            .with_bash_allow(vec!["cat".into()]);

        let denied = exec.execute(&call(TOOL_BASH, json!({ "command": "cat < input.txt" })));

        assert_eq!(denied.status, ToolStatus::Error);
        assert!(denied.content.contains("input.txt"));
        assert!(denied.content.contains("file read is not granted"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bash_refuses_shell_control_operators_before_execution() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root).with_bash_allow(vec!["echo".into()]);

        for command in [
            "echo ok; touch owned.txt",
            "echo ok && touch owned.txt",
            "echo ok | touch owned.txt",
            "echo ok\n touch owned.txt",
        ] {
            let denied = exec.execute(&call(TOOL_BASH, json!({ "command": command })));
            assert_eq!(denied.status, ToolStatus::Error, "command: {command}");
            assert!(denied.content.contains("command refused"));
        }
        assert!(!root.join("owned.txt").exists());

        let quoted = exec.execute(&call(
            TOOL_BASH,
            json!({ "command": "echo 'a; b | c && d (x)'" }),
        ));
        assert_eq!(quoted.status, ToolStatus::Ok);
        assert!(quoted.content.contains("a; b | c && d (x)"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bash_refuses_command_substitution_before_execution() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root).with_bash_allow(vec!["echo".into()]);

        let dollar = exec.execute(&call(
            TOOL_BASH,
            json!({ "command": "echo $(touch owned.txt)" }),
        ));
        let backticks = exec.execute(&call(
            TOOL_BASH,
            json!({ "command": "echo `touch backtick-owned.txt`" }),
        ));

        assert_eq!(dollar.status, ToolStatus::Error);
        assert!(dollar.content.contains("command substitution"));
        assert_eq!(backticks.status, ToolStatus::Error);
        assert!(backticks.content.contains("command substitution"));
        assert!(!root.join("owned.txt").exists());
        assert!(!root.join("backtick-owned.txt").exists());

        let literal = exec.execute(&call(
            TOOL_BASH,
            json!({ "command": "echo '$(touch literal.txt)'" }),
        ));
        assert_eq!(literal.status, ToolStatus::Ok);
        assert!(literal.content.contains("$(touch literal.txt)"));
        assert!(!root.join("literal.txt").exists());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bash_refuses_dynamic_shell_expansion_before_execution() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root).with_bash_allow(vec!["echo".into()]);

        for command in ["echo $HOME", "echo *.rs", "echo {a,b}", "echo ~/secret"] {
            let denied = exec.execute(&call(TOOL_BASH, json!({ "command": command })));
            assert_eq!(denied.status, ToolStatus::Error, "command: {command}");
            assert!(denied.content.contains("command refused"));
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bash_refuses_out_of_workspace_path_arguments() {
        let root = temp_root();
        let exec = FileToolExecutor::new(&root).with_bash_allow(vec!["echo".into()]);

        for command in [
            "echo ../secret",
            "echo /tmp/secret",
            "echo --input=../secret",
        ] {
            let denied = exec.execute(&call(TOOL_BASH, json!({ "command": command })));
            assert_eq!(denied.status, ToolStatus::Error, "command: {command}");
            assert!(denied.content.contains("must stay within the workspace"));
        }
        std::fs::remove_dir_all(&root).ok();
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
