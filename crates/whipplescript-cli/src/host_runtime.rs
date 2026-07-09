//! Persistent native host facade for governed WhippleScript turns.
//!
//! The facade owns policy admission, instance identity, the brokered model/tool
//! loop, transcript persistence, and evidence projection. Embedding products
//! provide only opaque-reference resolvers. Secrets and resource bodies are
//! resolved after admission and never enter the host command or receipt.

use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use whipplescript_kernel::coerce_native::CoerceProvider;
pub use whipplescript_kernel::harness_loop::ToolCall;
use whipplescript_kernel::harness_loop::{
    BrokeredTurnInput, ChatMessage, ImageBlock, NoopCompactor, ToolExecutor, ToolOutcome, ToolSpec,
    ToolStatus,
};
use whipplescript_kernel::harness_model::MessagesApiClient;
use whipplescript_kernel::sansio::{HostDriver, HttpResponse, IoRequest, IoResult, TransportError};
use whipplescript_kernel::{
    idempotency_key, AgentThreadSeed, BrokeredTurnContext, ProgramVersionInput, RuntimeKernel,
};
use whipplescript_parser::IrProgram;
use whipplescript_store::{
    EffectCancellationRequest, EvidenceRecord, NewEffect, NewEvent, RuleCommit, SqliteStore,
    StoreError,
};

use crate::host_protocol::{
    EventPosition, ForkInstanceCommand, ForkedInstance, LabeledRuntimeEvent, OpenInstanceCommand,
    OpenedInstance, PolicyEpochRef, ProtocolError, ProviderBindingRef, ResourceRef,
    StartTurnCommand, TurnReceipt, TurnStatus, HOST_PROTOCOL,
};
use crate::ifc::VerifiedEnvelope;

/// A package version resolved from WhippleScript's package store.
///
/// Tool schemas come from the pinned package. The embedding host cannot add a
/// tool at turn time; it only implements the resource operations behind tools
/// the package already declares.
#[derive(Clone, Debug)]
pub struct ResolvedPackage {
    version_ref: String,
    source_hash: String,
    ir_hash: String,
    agent: String,
    system_prompt: String,
    tools: Vec<ToolSpec>,
    max_steps: usize,
    program: IrProgram,
}

impl ResolvedPackage {
    /// Compile the exact pinned WhippleScript source the host resolved. Package
    /// identity is derived from those bytes; the resulting IR is retained so
    /// runtime admission can execute WhippleScript's IFC checker under the
    /// verified governance envelope.
    #[allow(clippy::too_many_arguments)]
    pub fn compile(
        version_ref: impl Into<String>,
        source: &str,
        root: Option<&str>,
        agent: impl Into<String>,
        system_prompt: impl Into<String>,
        tools: Vec<ToolSpec>,
        max_steps: usize,
    ) -> Result<Self, String> {
        let compiled = whipplescript_parser::compile_program_with_root(source, root);
        let program = compiled.ir.ok_or_else(|| {
            compiled
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        let agent = agent.into();
        let system_prompt = system_prompt.into();
        let tool_identity = tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect::<Vec<_>>();
        let package_identity = json!({
            "source": source,
            "root": root,
            "agent": &agent,
            "system_prompt": &system_prompt,
            "tools": tool_identity,
            "max_steps": max_steps,
        });
        let source_hash = sha256_hex(package_identity.to_string().as_bytes());
        let ir_hash = sha256_hex(
            format!("{}:{}:{}", source_hash, program.workflow, HOST_PROTOCOL).as_bytes(),
        );
        Ok(Self {
            version_ref: version_ref.into(),
            source_hash,
            ir_hash,
            agent,
            system_prompt,
            tools,
            max_steps,
            program,
        })
    }
}

/// Resolve an immutable WhippleScript package version by opaque reference.
pub trait PackageResolver {
    fn resolve_package(&self, version_ref: &str) -> Result<ResolvedPackage, String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelProvider {
    OpenAi,
    Anthropic,
    Codex,
}

/// Ephemeral provider material. Its `Debug` implementation is deliberately
/// redacted and the value is never serialized by this module.
pub struct ResolvedProviderBinding {
    provider: ModelProvider,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u64,
    timeout: Duration,
    codex_account_id: Option<String>,
    codex_session_id: Option<String>,
}

impl ResolvedProviderBinding {
    pub fn new(
        provider: ModelProvider,
        api_key: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
        max_tokens: u64,
        timeout: Duration,
    ) -> Self {
        Self {
            provider,
            api_key: api_key.into(),
            model: model.into(),
            base_url: base_url.into(),
            max_tokens,
            timeout,
            codex_account_id: None,
            codex_session_id: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_codex(
        access_token: impl Into<String>,
        account_id: impl Into<String>,
        session_id: impl Into<String>,
        model: impl Into<String>,
        base_url: impl Into<String>,
        max_tokens: u64,
        timeout: Duration,
    ) -> Self {
        Self {
            provider: ModelProvider::Codex,
            api_key: access_token.into(),
            model: model.into(),
            base_url: base_url.into(),
            max_tokens,
            timeout,
            codex_account_id: Some(account_id.into()),
            codex_session_id: Some(session_id.into()),
        }
    }

    fn validate(&self) -> Result<(), HostRuntimeError> {
        if self.api_key.trim().is_empty()
            || self.model.trim().is_empty()
            || self.base_url.trim().is_empty()
            || self.max_tokens == 0
        {
            return Err(HostRuntimeError::Resolver(
                "provider binding is incomplete".to_owned(),
            ));
        }
        if self.provider == ModelProvider::Codex
            && (self.codex_account_id.as_deref().is_none_or(str::is_empty)
                || self.codex_session_id.as_deref().is_none_or(str::is_empty))
        {
            return Err(HostRuntimeError::Resolver(
                "Codex provider binding has no account/session identity".to_owned(),
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for ResolvedProviderBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedProviderBinding")
            .field("provider", &self.provider)
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("max_tokens", &self.max_tokens)
            .field("timeout", &self.timeout)
            .field("codex_account_id", &self.codex_account_id)
            .field(
                "codex_session_id",
                &self.codex_session_id.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Resolve credential bytes only after WhippleScript has admitted the policy,
/// provider binding, and placement ceiling. Resolver errors must not contain
/// secret material.
pub trait SecretResolver {
    fn resolve_provider(
        &self,
        binding: &ProviderBindingRef,
        placement_ceiling_ref: &str,
    ) -> Result<ResolvedProviderBinding, String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedImage {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// The host implementation behind package-declared resource tools.
///
/// Every call receives only the resource references admitted for this turn.
/// WhippleScript checks the tool name against the pinned package before invoking
/// the resolver, so neither model nor host can widen the tool surface in flight.
pub trait ResourceResolver {
    fn resolve_image(&self, image: &ResourceRef) -> Result<ResolvedImage, String>;

    fn execute_tool(
        &self,
        admitted_resources: &[ResourceRef],
        call: &ToolCall,
    ) -> Result<String, String>;
}

/// WhippleScript-owned native implementation of the workspace capability used
/// by embedding desktop hosts. GaugeDesk supplies only the root and any
/// read-only subtrees; WhippleScript parses tool arguments, confines paths,
/// rejects symlink traversal, and performs the operation.
pub struct NativeWorkspaceResolver {
    root: PathBuf,
    read_only: Vec<PathBuf>,
    max_output_bytes: usize,
}

impl NativeWorkspaceResolver {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root
            .as_ref()
            .canonicalize()
            .map_err(|error| format!("cannot open workspace capability: {error}"))?;
        if !root.is_dir() {
            return Err("workspace capability root is not a directory".to_owned());
        }
        Ok(Self {
            root,
            read_only: Vec::new(),
            max_output_bytes: 50_000,
        })
    }

    pub fn read_only(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Result<Self, String> {
        self.read_only = paths
            .into_iter()
            .map(|path| normalize_relative(&path))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self)
    }

    fn resolve(&self, path: &str, write: bool) -> Result<PathBuf, String> {
        let relative = normalize_relative(Path::new(path))?;
        if write
            && self
                .read_only
                .iter()
                .any(|protected| relative.starts_with(protected))
        {
            return Err(format!("workspace path `{path}` is read-only"));
        }
        let mut resolved = self.root.clone();
        for component in relative.components() {
            let Component::Normal(segment) = component else {
                return Err(format!("workspace path `{path}` escapes its capability"));
            };
            resolved.push(segment);
            if let Ok(metadata) = fs::symlink_metadata(&resolved) {
                if metadata.file_type().is_symlink() {
                    return Err(format!("workspace path `{path}` traverses a symlink"));
                }
            }
        }
        Ok(resolved)
    }

    fn cap(&self, text: String) -> String {
        if text.len() <= self.max_output_bytes {
            return text;
        }
        let mut boundary = self.max_output_bytes;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!(
            "{}\n… output truncated by WhippleScript …",
            &text[..boundary]
        )
    }

    fn read(&self, arguments: &Value) -> Result<String, String> {
        let path = string_argument(arguments, "path")?;
        let resolved = self.resolve(path, false)?;
        let text = fs::read_to_string(&resolved)
            .map_err(|error| format!("cannot read workspace path `{path}`: {error}"))?;
        let offset = arguments
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(2_000) as usize;
        let lines = text
            .lines()
            .skip(offset - 1)
            .take(limit)
            .enumerate()
            .map(|(index, line)| format!("{}: {line}", offset + index))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(self.cap(lines))
    }

    fn write(&self, arguments: &Value) -> Result<String, String> {
        let path = string_argument(arguments, "path")?;
        let content = string_argument(arguments, "content")?;
        let resolved = self.resolve(path, true)?;
        if let Some(parent) = resolved.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create parent for `{path}`: {error}"))?;
            reject_symlinks_between(&self.root, parent, path)?;
        }
        fs::write(&resolved, content)
            .map_err(|error| format!("cannot write workspace path `{path}`: {error}"))?;
        Ok(format!("wrote {} bytes to {path}", content.len()))
    }

    fn edit(&self, arguments: &Value) -> Result<String, String> {
        let path = string_argument(arguments, "path")?;
        let resolved = self.resolve(path, true)?;
        let mut text = fs::read_to_string(&resolved)
            .map_err(|error| format!("cannot edit workspace path `{path}`: {error}"))?;
        let edits = arguments
            .get("edits")
            .and_then(Value::as_array)
            .ok_or_else(|| "`edits` must be an array".to_owned())?;
        for edit in edits {
            let old = string_argument(edit, "oldText")?;
            let new = string_argument(edit, "newText")?;
            if old.is_empty() {
                return Err("edit oldText must not be empty".to_owned());
            }
            if text.matches(old).count() != 1 {
                return Err("edit oldText must match exactly once".to_owned());
            }
            text = text.replacen(old, new, 1);
        }
        fs::write(&resolved, text)
            .map_err(|error| format!("cannot edit workspace path `{path}`: {error}"))?;
        Ok(format!("applied {} edit(s) to {path}", edits.len()))
    }

    fn list(&self, arguments: &Value) -> Result<String, String> {
        let path = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let resolved = self.resolve(path, false)?;
        let mut names = fs::read_dir(&resolved)
            .map_err(|error| format!("cannot list workspace path `{path}`: {error}"))?
            .filter_map(Result::ok)
            .map(|entry| {
                let mut name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                    name.push('/');
                }
                name
            })
            .collect::<Vec<_>>();
        names.sort();
        Ok(self.cap(names.join("\n")))
    }

    fn find(&self, arguments: &Value) -> Result<String, String> {
        let path = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let pattern = string_argument(arguments, "pattern")?;
        let resolved = self.resolve(path, false)?;
        let mut matches = Vec::new();
        walk_workspace(&self.root, &resolved, &mut |relative, _| {
            if wildcard_matches(pattern, relative) {
                matches.push(relative.to_owned());
            }
            matches.len() < 5_000
        })?;
        matches.sort();
        Ok(self.cap(matches.join("\n")))
    }

    fn grep(&self, arguments: &Value) -> Result<String, String> {
        let path = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let pattern = string_argument(arguments, "pattern")?;
        let matcher = regex::Regex::new(pattern).ok();
        let resolved = self.resolve(path, false)?;
        let mut matches = Vec::new();
        walk_workspace(&self.root, &resolved, &mut |relative, absolute| {
            let Ok(text) = fs::read_to_string(absolute) else {
                return true;
            };
            for (line, content) in text.lines().enumerate() {
                let hit = matcher
                    .as_ref()
                    .map(|regex| regex.is_match(content))
                    .unwrap_or_else(|| content.contains(pattern));
                if hit {
                    matches.push(format!("{relative}:{}:{content}", line + 1));
                    if matches.len() >= 5_000 {
                        return false;
                    }
                }
            }
            true
        })?;
        Ok(self.cap(matches.join("\n")))
    }
}

impl ResourceResolver for NativeWorkspaceResolver {
    fn resolve_image(&self, image: &ResourceRef) -> Result<ResolvedImage, String> {
        let locator = image.selector.as_deref().unwrap_or(&image.handle);
        let path = self.resolve(locator, false)?;
        let bytes =
            fs::read(&path).map_err(|error| format!("cannot read image `{locator}`: {error}"))?;
        let media_type = match path.extension().and_then(|extension| extension.to_str()) {
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            _ => return Err("unsupported image media type".to_owned()),
        };
        Ok(ResolvedImage {
            media_type: media_type.to_owned(),
            bytes,
        })
    }

    fn execute_tool(
        &self,
        admitted_resources: &[ResourceRef],
        call: &ToolCall,
    ) -> Result<String, String> {
        if !admitted_resources
            .iter()
            .any(|resource| resource.kind == "file_store")
        {
            return Err("turn has no admitted file-store capability".to_owned());
        }
        match call.name.as_str() {
            "read" => self.read(&call.arguments),
            "write" => self.write(&call.arguments),
            "edit" => self.edit(&call.arguments),
            "ls" => self.list(&call.arguments),
            "find" => self.find(&call.arguments),
            "grep" => self.grep(&call.arguments),
            _ => Err("tool has no native workspace implementation".to_owned()),
        }
    }
}

/// The model-facing workspace tools owned by WhippleScript. An embedding host
/// selects whether mutation is present; it cannot redefine their schemas or
/// execution semantics.
pub fn native_workspace_tool_specs(writable: bool) -> Vec<ToolSpec> {
    let mut tools = vec![
        tool_spec(
            "read",
            "Read a workspace text file.",
            json!({
                "type": "object", "properties": {
                    "path": { "type": "string" }, "offset": { "type": "integer" },
                    "limit": { "type": "integer" }
                }, "required": ["path"], "additionalProperties": false
            }),
        ),
        tool_spec(
            "grep",
            "Search text in workspace files.",
            json!({
                "type": "object", "properties": {
                    "pattern": { "type": "string" }, "path": { "type": "string" }
                }, "required": ["pattern"], "additionalProperties": false
            }),
        ),
        tool_spec(
            "find",
            "Find workspace paths by wildcard pattern.",
            json!({
                "type": "object", "properties": {
                    "pattern": { "type": "string" }, "path": { "type": "string" }
                }, "required": ["pattern"], "additionalProperties": false
            }),
        ),
        tool_spec(
            "ls",
            "List a workspace directory.",
            json!({
                "type": "object", "properties": { "path": { "type": "string" } },
                "additionalProperties": false
            }),
        ),
    ];
    if writable {
        tools.extend([
            tool_spec(
                "write",
                "Create or replace a workspace text file.",
                json!({
                    "type": "object", "properties": {
                        "path": { "type": "string" }, "content": { "type": "string" }
                    }, "required": ["path", "content"], "additionalProperties": false
                }),
            ),
            tool_spec(
                "edit",
                "Apply exact, unique string replacements.",
                json!({
                    "type": "object", "properties": {
                        "path": { "type": "string" }, "edits": { "type": "array", "items": {
                            "type": "object", "properties": {
                                "oldText": { "type": "string" }, "newText": { "type": "string" }
                            }, "required": ["oldText", "newText"], "additionalProperties": false
                        }}
                    }, "required": ["path", "edits"], "additionalProperties": false
                }),
            ),
        ]);
    }
    tools
}

fn tool_spec(name: &str, description: &str, input_schema: Value) -> ToolSpec {
    ToolSpec {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema,
    }
}

fn string_argument<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required string argument `{name}`"))
}

fn normalize_relative(path: &Path) -> Result<PathBuf, String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "workspace path `{}` escapes its capability",
                    path.display()
                ));
            }
        }
    }
    Ok(normalized)
}

fn reject_symlinks_between(root: &Path, target: &Path, display: &str) -> Result<(), String> {
    let relative = target
        .strip_prefix(root)
        .map_err(|_| format!("workspace path `{display}` escapes its capability"))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        if fs::symlink_metadata(&current)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(format!("workspace path `{display}` traverses a symlink"));
        }
    }
    Ok(())
}

fn walk_workspace(
    root: &Path,
    start: &Path,
    visit: &mut dyn FnMut(&str, &Path) -> bool,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(start)
        .map_err(|error| format!("cannot inspect workspace path: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("workspace traversal reached a symlink".to_owned());
    }
    if metadata.is_file() {
        let relative = start
            .strip_prefix(root)
            .map_err(|_| "workspace traversal escaped its capability".to_owned())?
            .to_string_lossy();
        let _ = visit(&relative, start);
        return Ok(());
    }
    let mut pending = vec![start.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut entries = fs::read_dir(&directory)
            .map_err(|error| format!("cannot walk workspace: {error}"))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|error| format!("cannot inspect workspace entry: {error}"))?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(path);
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "workspace traversal escaped its capability".to_owned())?
                .to_string_lossy();
            if !visit(&relative, &path) {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn wildcard_matches(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let mut previous = vec![false; text.len() + 1];
    previous[0] = true;
    for &token in pattern {
        let mut current = vec![false; text.len() + 1];
        if token == b'*' {
            current[0] = previous[0];
        }
        for index in 1..=text.len() {
            current[index] = match token {
                b'*' => previous[index] || current[index - 1],
                b'?' => previous[index - 1],
                byte => previous[index - 1] && byte == text[index - 1],
            };
        }
        previous = current;
    }
    previous[text.len()]
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
    pub result: Option<String>,
    pub ok: Option<bool>,
}

/// The content projection WhippleScript has admitted for its embedding host.
///
/// The projection is derived from WhippleScript's durable transcript, carries
/// the same IFC join label as the evidence stream, and is the only supported
/// way for a product shell to obtain assistant/tool content. Embedding hosts do
/// not inspect the runtime store or recreate transcript-folding semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabeledTurnOutput {
    pub output_handle: Option<String>,
    pub label_ref: String,
    pub assistant_text: String,
    pub tool_calls: Vec<ProjectedToolCall>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnExecution {
    pub events: Vec<LabeledRuntimeEvent>,
    pub receipt: TurnReceipt,
    pub output: Option<LabeledTurnOutput>,
}

/// Out-of-band cooperative cancellation capability for one admitted host
/// command. It opens an independent store connection, so an embedding UI can
/// request cancellation while the runtime-owning thread is blocked in provider
/// I/O. The owned loop observes it between model rounds.
#[derive(Clone, Debug)]
pub struct HostCancellationHandle {
    store_path: PathBuf,
    instance_ref: String,
    command_id: String,
}

impl HostCancellationHandle {
    pub fn request(&self) -> Result<(), HostRuntimeError> {
        let mut store = SqliteStore::open(&self.store_path).map_err(HostRuntimeError::Store)?;
        let idempotency = idempotency_key(&[
            &self.instance_ref,
            &self.command_id,
            "host-cancellation-request",
        ]);
        store
            .request_effect_cancellation(EffectCancellationRequest {
                instance_id: &self.instance_ref,
                effect_id: &self.command_id,
                revision_id: None,
                reason: Some("embedding host requested cancellation"),
                requested_by: "embedding-host",
                causation_event_id: None,
                idempotency_key: Some(&idempotency),
            })
            .map(|_| ())
            .map_err(HostRuntimeError::Store)
    }
}

/// A persistent, policy-bound native WhippleScript runtime.
pub struct GovernedHostRuntime {
    kernel: RuntimeKernel<SqliteStore>,
    store_path: PathBuf,
    policy: PolicyEpochRef,
    envelope: VerifiedEnvelope,
}

impl GovernedHostRuntime {
    /// Open or reopen a native runtime store and bind this facade to one signed,
    /// immutable policy epoch.
    pub fn open(
        store_path: impl AsRef<Path>,
        epoch: u64,
        signed_envelope: &str,
    ) -> Result<Self, HostRuntimeError> {
        let envelope = VerifiedEnvelope::verify_signed_text(signed_envelope)
            .map_err(HostRuntimeError::PolicyRejected)?;
        Self::open_verified(store_path, epoch, envelope)
    }

    /// Open an embedding runtime under an externally signed governance
    /// envelope. The verifier is an explicit capability held by the governance
    /// authority; no process-global admin flag participates.
    pub fn open_with_verifier<V: crate::gov::GovernanceAttestationVerifier + ?Sized>(
        store_path: impl AsRef<Path>,
        epoch: u64,
        signed_envelope: &str,
        verifier: &V,
    ) -> Result<Self, HostRuntimeError> {
        let envelope = VerifiedEnvelope::verify_signed_text_with(signed_envelope, verifier)
            .map_err(HostRuntimeError::PolicyRejected)?;
        Self::open_verified(store_path, epoch, envelope)
    }

    fn open_verified(
        store_path: impl AsRef<Path>,
        epoch: u64,
        envelope: VerifiedEnvelope,
    ) -> Result<Self, HostRuntimeError> {
        let policy = PolicyEpochRef::from_verified(epoch, &envelope)?;
        let store_path = store_path.as_ref().to_path_buf();
        let store = SqliteStore::open(&store_path).map_err(HostRuntimeError::Store)?;
        Ok(Self {
            kernel: RuntimeKernel::new(store),
            store_path,
            policy,
            envelope,
        })
    }

    pub fn policy_ref(&self) -> &PolicyEpochRef {
        &self.policy
    }

    /// The latest durable coordinate for an instance. Hosts use this to bind a
    /// fork to one explicit source point rather than an implicit moving head.
    pub fn current_position(&self, instance_ref: &str) -> Result<EventPosition, HostRuntimeError> {
        let events = self
            .kernel
            .store()
            .list_events(instance_ref)
            .map_err(HostRuntimeError::Store)?;
        let latest = events
            .last()
            .ok_or_else(|| HostRuntimeError::UnknownInstance(instance_ref.to_owned()))?;
        Ok(EventPosition {
            instance_ref: instance_ref.to_owned(),
            sequence: positive_sequence(latest.sequence)?,
        })
    }

    /// Mint the out-of-band cancel capability for a command before driving it.
    /// The handle contains no provider secret or resource body.
    pub fn cancellation_handle(
        &self,
        instance_ref: impl Into<String>,
        command_id: impl Into<String>,
    ) -> HostCancellationHandle {
        HostCancellationHandle {
            store_path: self.store_path.clone(),
            instance_ref: instance_ref.into(),
            command_id: command_id.into(),
        }
    }

    /// Create the durable WhippleScript instance for a chat. The returned opaque
    /// instance reference is the value the host persists and uses on every turn.
    pub fn open_instance<P: PackageResolver + ?Sized>(
        &mut self,
        command: &OpenInstanceCommand,
        packages: &P,
    ) -> Result<OpenedInstance, HostRuntimeError> {
        command.validate()?;
        self.require_policy(&command.policy)?;
        let package = packages
            .resolve_package(&command.package_version_ref)
            .map_err(HostRuntimeError::Resolver)?;
        validate_package(&package, &command.package_version_ref)?;
        self.check_package_ifc(&package)?;
        if let Some(opened) = self.replayed_open_instance(command, &package)? {
            return Ok(opened);
        }

        let version = self
            .kernel
            .create_program_version(ProgramVersionInput {
                program_name: &package.agent,
                source_hash: &package.source_hash,
                ir_hash: &package.ir_hash,
                compiler_version: HOST_PROTOCOL,
            })
            .map_err(HostRuntimeError::Store)?;
        let metadata = InstanceMetadata {
            protocol: HOST_PROTOCOL.to_owned(),
            package_version_ref: command.package_version_ref.clone(),
            policy: command.policy.clone(),
        };
        let input_json = serde_json::to_string(&metadata).map_err(HostRuntimeError::Json)?;
        let instance_ref = self
            .kernel
            .create_instance(&version, &input_json)
            .map_err(HostRuntimeError::Store)?;
        let payload = json!({
            "request_id": command.request_id,
            "package_version_ref": command.package_version_ref,
            "policy": command.policy,
        })
        .to_string();
        let opened = self
            .kernel
            .store()
            .append_event(NewEvent {
                instance_id: &instance_ref,
                event_type: "host.instance.opened",
                payload_json: &payload,
                source: "host-runtime",
                causation_id: None,
                correlation_id: Some(&command.request_id),
                idempotency_key: Some(&idempotency_key(&[
                    &instance_ref,
                    &command.request_id,
                    "host-instance-opened",
                ])),
            })
            .map_err(HostRuntimeError::Store)?;
        let result = OpenedInstance {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: command.request_id.clone(),
            instance_ref: instance_ref.clone(),
            package_version_ref: command.package_version_ref.clone(),
            policy: command.policy.clone(),
            opened_at: EventPosition {
                instance_ref,
                sequence: positive_sequence(opened.sequence)?,
            },
        };
        result.validate_for(command)?;
        Ok(result)
    }

    /// Fork the source runtime's live agent thread into a distinct target
    /// instance. The source coordinate, package, and policy are all validated;
    /// the target records an idempotent seed event rather than copying a store
    /// file or pretending to have executed the source effects.
    pub fn fork_instance_from<P: PackageResolver + ?Sized>(
        &mut self,
        source_runtime: &GovernedHostRuntime,
        command: &ForkInstanceCommand,
        packages: &P,
    ) -> Result<ForkedInstance, HostRuntimeError> {
        command.validate()?;
        self.require_policy(&command.policy)?;
        source_runtime.require_policy(&command.policy)?;

        let package = packages
            .resolve_package(&command.package_version_ref)
            .map_err(HostRuntimeError::Resolver)?;
        validate_package(&package, &command.package_version_ref)?;
        self.check_package_ifc(&package)?;
        source_runtime.check_package_ifc(&package)?;
        source_runtime.validate_instance_binding(
            &command.source.instance_ref,
            &command.package_version_ref,
            &command.policy,
            packages,
        )?;

        let current = source_runtime.current_position(&command.source.instance_ref)?;
        if command.source.sequence > current.sequence {
            return Err(HostRuntimeError::Protocol(ProtocolError::Mismatch(
                "fork source position",
            )));
        }
        let running = source_runtime
            .kernel
            .store()
            .list_effects(&command.source.instance_ref)
            .map_err(HostRuntimeError::Store)?
            .into_iter()
            .any(|effect| effect.status == "running");
        if running {
            return Err(HostRuntimeError::Incomplete(format!(
                "source instance {} is not quiescent",
                command.source.instance_ref
            )));
        }

        let target_command = command.target_open_command();
        let target = self.open_instance(&target_command, packages)?;
        if let Some(replayed) = self.replayed_fork_instance(command, &target)? {
            return Ok(replayed);
        }
        if target.instance_ref == command.source.instance_ref {
            return Err(HostRuntimeError::Protocol(ProtocolError::Mismatch(
                "fork target identity",
            )));
        }

        let messages = source_runtime
            .kernel
            .snapshot_agent_thread(
                &command.source.instance_ref,
                &package.agent,
                Some(command.source.sequence as i64),
            )
            .map_err(HostRuntimeError::Store)?;
        self.kernel
            .seed_agent_thread(AgentThreadSeed {
                instance_id: &target.instance_ref,
                agent: &package.agent,
                messages: &messages,
                source_instance_id: &command.source.instance_ref,
                source_sequence: command.source.sequence as i64,
                idempotency_key: &idempotency_key(&[
                    &target.instance_ref,
                    &command.request_id,
                    "host-instance-thread-seed",
                ]),
            })
            .map_err(HostRuntimeError::Store)?;
        let payload = json!({
            "request_id": command.request_id,
            "source": command.source,
            "target_request_id": command.target_request_id,
            "package_version_ref": command.package_version_ref,
            "policy": command.policy,
        })
        .to_string();
        let event = self
            .kernel
            .store()
            .append_event(NewEvent {
                instance_id: &target.instance_ref,
                event_type: "host.instance.forked",
                payload_json: &payload,
                source: "host-runtime",
                causation_id: None,
                correlation_id: Some(&command.request_id),
                idempotency_key: Some(&idempotency_key(&[
                    &target.instance_ref,
                    &command.request_id,
                    "host-instance-forked",
                ])),
            })
            .map_err(HostRuntimeError::Store)?;
        let target_instance_ref = target.instance_ref.clone();
        let result = ForkedInstance {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: command.request_id.clone(),
            source: command.source.clone(),
            target,
            forked_at: EventPosition {
                instance_ref: target_instance_ref,
                sequence: positive_sequence(event.sequence)?,
            },
        };
        result.validate_for(command)?;
        Ok(result)
    }

    fn replayed_fork_instance(
        &self,
        command: &ForkInstanceCommand,
        target: &OpenedInstance,
    ) -> Result<Option<ForkedInstance>, HostRuntimeError> {
        let events = self
            .kernel
            .store()
            .list_events(&target.instance_ref)
            .map_err(HostRuntimeError::Store)?;
        let Some(event) = events.iter().find(|event| {
            event.event_type == "host.instance.forked"
                && serde_json::from_str::<Value>(&event.payload_json)
                    .ok()
                    .and_then(|payload| {
                        payload
                            .get("request_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .as_deref()
                    == Some(command.request_id.as_str())
        }) else {
            return Ok(None);
        };
        let payload: Value =
            serde_json::from_str(&event.payload_json).map_err(HostRuntimeError::Json)?;
        let result = ForkedInstance {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: required_string(&payload, "request_id")?,
            source: serde_json::from_value(payload["source"].clone())
                .map_err(HostRuntimeError::Json)?,
            target: target.clone(),
            forked_at: EventPosition {
                instance_ref: target.instance_ref.clone(),
                sequence: positive_sequence(event.sequence)?,
            },
        };
        result.validate_for(command)?;
        Ok(Some(result))
    }

    fn replayed_open_instance(
        &self,
        command: &OpenInstanceCommand,
        package: &ResolvedPackage,
    ) -> Result<Option<OpenedInstance>, HostRuntimeError> {
        for instance in self
            .kernel
            .store()
            .list_instances()
            .map_err(HostRuntimeError::Store)?
        {
            let events = self
                .kernel
                .store()
                .list_events(&instance.instance_id)
                .map_err(HostRuntimeError::Store)?;
            for event in events {
                if event.event_type != "host.instance.opened" {
                    continue;
                }
                let payload: Value =
                    serde_json::from_str(&event.payload_json).map_err(HostRuntimeError::Json)?;
                if payload.get("request_id").and_then(Value::as_str)
                    != Some(command.request_id.as_str())
                {
                    continue;
                }
                let opened = OpenedInstance {
                    protocol: HOST_PROTOCOL.to_owned(),
                    request_id: command.request_id.clone(),
                    instance_ref: instance.instance_id.clone(),
                    package_version_ref: required_string(&payload, "package_version_ref")?,
                    policy: serde_json::from_value(payload["policy"].clone())
                        .map_err(HostRuntimeError::Json)?,
                    opened_at: EventPosition {
                        instance_ref: instance.instance_id.clone(),
                        sequence: positive_sequence(event.sequence)?,
                    },
                };
                opened.validate_for(command)?;
                let version = self
                    .kernel
                    .store()
                    .get_program_version(&instance.version_id)
                    .map_err(HostRuntimeError::Store)?
                    .ok_or_else(|| HostRuntimeError::UnknownInstance(instance.instance_id))?;
                if version.source_hash != package.source_hash || version.ir_hash != package.ir_hash
                {
                    return Err(HostRuntimeError::Protocol(ProtocolError::Mismatch(
                        "replayed package content",
                    )));
                }
                return Ok(Some(opened));
            }
        }
        Ok(None)
    }

    /// Run a turn through WhippleScript's owned brokered loop using the native
    /// HTTP transport.
    pub fn run_turn<P, S, R>(
        &mut self,
        command: &StartTurnCommand,
        packages: &P,
        secrets: &S,
        resources: &R,
    ) -> Result<TurnExecution, HostRuntimeError>
    where
        P: PackageResolver + ?Sized,
        S: SecretResolver + ?Sized,
        R: ResourceResolver + ?Sized,
    {
        let binding = self.admit_and_resolve(command, packages, secrets)?;
        let driver = NativeHttpDriver::new(binding.timeout);
        self.run_admitted_turn(command, packages, resources, binding, &driver)
    }

    /// The same governed path with a caller-supplied sans-I/O driver. Native
    /// tests and remote hosts use this to drive the exact machine without a
    /// second turn implementation.
    pub fn run_turn_with_driver<P, S, R, H>(
        &mut self,
        command: &StartTurnCommand,
        packages: &P,
        secrets: &S,
        resources: &R,
        driver: &H,
    ) -> Result<TurnExecution, HostRuntimeError>
    where
        P: PackageResolver + ?Sized,
        S: SecretResolver + ?Sized,
        R: ResourceResolver + ?Sized,
        H: HostDriver,
    {
        let binding = self.admit_and_resolve(command, packages, secrets)?;
        self.run_admitted_turn(command, packages, resources, binding, driver)
    }

    fn admit_and_resolve<P, S>(
        &self,
        command: &StartTurnCommand,
        packages: &P,
        secrets: &S,
    ) -> Result<ResolvedProviderBinding, HostRuntimeError>
    where
        P: PackageResolver + ?Sized,
        S: SecretResolver + ?Sized,
    {
        command.validate()?;
        self.require_policy(&command.policy)?;
        self.require_governed(&command.provider_binding.binding_id)?;
        self.require_governed(&command.placement_ceiling_ref)?;
        for resource in command.resources.iter().chain(command.input.images.iter()) {
            self.require_governed(&resource.handle)?;
        }
        self.validate_instance(command, packages)?;
        let binding = secrets
            .resolve_provider(&command.provider_binding, &command.placement_ceiling_ref)
            .map_err(HostRuntimeError::Resolver)?;
        binding.validate()?;
        Ok(binding)
    }

    fn run_admitted_turn<P, R, H>(
        &mut self,
        command: &StartTurnCommand,
        packages: &P,
        resources: &R,
        binding: ResolvedProviderBinding,
        driver: &H,
    ) -> Result<TurnExecution, HostRuntimeError>
    where
        P: PackageResolver + ?Sized,
        R: ResourceResolver + ?Sized,
        H: HostDriver,
    {
        if let Some(execution) = self.stored_execution(command)? {
            return Ok(execution);
        }
        let package = packages
            .resolve_package(&command.package_version_ref)
            .map_err(HostRuntimeError::Resolver)?;
        validate_package(&package, &command.package_version_ref)?;
        self.check_package_ifc(&package)?;
        let command_json = serde_json::to_string(command).map_err(HostRuntimeError::Json)?;
        let effects = self
            .kernel
            .store()
            .list_effects(&command.instance_ref)
            .map_err(HostRuntimeError::Store)?;
        match effects
            .iter()
            .find(|effect| effect.effect_id == command.command_id)
        {
            Some(effect) if effect.input_json != command_json => {
                return Err(HostRuntimeError::Protocol(ProtocolError::Mismatch(
                    "command id reused with different turn",
                )));
            }
            Some(effect) if is_terminal_effect(&effect.status) => {
                return self.finish_execution(command);
            }
            Some(_) => {}
            None => {
                self.kernel
                    .commit_rule(RuleCommit {
                        instance_id: &command.instance_ref,
                        rule: "host.turn",
                        trigger_event_id: None,
                        facts: &[],
                        consumed_fact_ids: &[],
                        effects: &[NewEffect {
                            effect_id: &command.command_id,
                            kind: "agent.tell",
                            target: Some(&package.agent),
                            input_json: &command_json,
                            status: "queued",
                            idempotency_key: &idempotency_key(&[
                                &command.instance_ref,
                                &command.command_id,
                                "host-turn-effect",
                            ]),
                            required_capabilities_json: "[]",
                            profile: None,
                            correlation_id: Some(&command.run_ref),
                            source_span_json: None,
                            timeout_seconds: None,
                        }],
                        dependencies: &[],
                        terminal: None,
                        idempotency_key: Some(&idempotency_key(&[
                            &command.instance_ref,
                            &command.command_id,
                            "host-turn-commit",
                        ])),
                    })
                    .map_err(HostRuntimeError::Store)?;
            }
        }

        let images = command
            .input
            .images
            .iter()
            .map(|image| {
                let resolved = resources
                    .resolve_image(image)
                    .map_err(HostRuntimeError::Resolver)?;
                if resolved.media_type.trim().is_empty() {
                    return Err(HostRuntimeError::Resolver(
                        "resolved image has no media type".to_owned(),
                    ));
                }
                Ok(ImageBlock {
                    media_type: resolved.media_type,
                    data_base64: base64_encode(&resolved.bytes),
                })
            })
            .collect::<Result<Vec<_>, HostRuntimeError>>()?;
        let executor = ResolverToolExecutor {
            offered: &package.tools,
            admitted_resources: &command.resources,
            resolver: resources,
        };
        let client = match binding.provider {
            ModelProvider::OpenAi => MessagesApiClient::new(
                CoerceProvider::OpenAi,
                binding.api_key,
                binding.model,
                binding.base_url,
                binding.max_tokens,
                Some(command.command_id.clone()),
            ),
            ModelProvider::Anthropic => MessagesApiClient::new(
                CoerceProvider::Anthropic,
                binding.api_key,
                binding.model,
                binding.base_url,
                binding.max_tokens,
                Some(command.command_id.clone()),
            ),
            ModelProvider::Codex => MessagesApiClient::new_codex(
                binding.api_key,
                binding.codex_account_id.unwrap_or_default(),
                binding.codex_session_id.unwrap_or_default(),
                binding.model,
                binding.base_url,
                binding.max_tokens,
                Some(command.command_id.clone()),
            ),
        };
        let input = BrokeredTurnInput {
            system: package.system_prompt,
            user: command.input.text.clone(),
            tools: package.tools.clone(),
            max_steps: package.max_steps,
            resume_from: Vec::new(),
            user_images: images,
            context_bundles: Vec::new(),
            pinned_skills: Vec::new(),
        };
        self.kernel
            .run_brokered_agent_turn(
                &BrokeredTurnContext {
                    instance_id: &command.instance_ref,
                    effect_id: &command.command_id,
                    agent: &package.agent,
                    profile: None,
                    thread_continue: true,
                },
                &client,
                &executor,
                driver,
                &NoopCompactor,
                &input,
            )
            .map_err(HostRuntimeError::Store)?;
        self.finish_execution(command)
    }

    fn validate_instance<P: PackageResolver + ?Sized>(
        &self,
        command: &StartTurnCommand,
        packages: &P,
    ) -> Result<(), HostRuntimeError> {
        self.validate_instance_binding(
            &command.instance_ref,
            &command.package_version_ref,
            &command.policy,
            packages,
        )
    }

    fn validate_instance_binding<P: PackageResolver + ?Sized>(
        &self,
        instance_ref: &str,
        package_version_ref: &str,
        policy: &PolicyEpochRef,
        packages: &P,
    ) -> Result<(), HostRuntimeError> {
        let instance = self
            .kernel
            .store()
            .get_instance(instance_ref)
            .map_err(HostRuntimeError::Store)?
            .ok_or_else(|| HostRuntimeError::UnknownInstance(instance_ref.to_owned()))?;
        let metadata: InstanceMetadata =
            serde_json::from_str(&instance.input_json).map_err(HostRuntimeError::Json)?;
        if metadata.protocol != HOST_PROTOCOL
            || metadata.package_version_ref != package_version_ref
            || metadata.policy != *policy
        {
            return Err(HostRuntimeError::Protocol(ProtocolError::Mismatch(
                "instance package/policy binding",
            )));
        }
        let package = packages
            .resolve_package(package_version_ref)
            .map_err(HostRuntimeError::Resolver)?;
        validate_package(&package, package_version_ref)?;
        self.check_package_ifc(&package)?;
        let version = self
            .kernel
            .store()
            .get_program_version(&instance.version_id)
            .map_err(HostRuntimeError::Store)?
            .ok_or_else(|| HostRuntimeError::UnknownInstance(instance_ref.to_owned()))?;
        if version.source_hash != package.source_hash || version.ir_hash != package.ir_hash {
            return Err(HostRuntimeError::Protocol(ProtocolError::Mismatch(
                "resolved package content",
            )));
        }
        Ok(())
    }

    fn finish_execution(
        &mut self,
        command: &StartTurnCommand,
    ) -> Result<TurnExecution, HostRuntimeError> {
        if let Some(execution) = self.stored_execution(command)? {
            return Ok(execution);
        }
        let run_id = idempotency_key(&[&command.instance_ref, &command.command_id, "brokered-run"]);
        let run = self
            .kernel
            .store()
            .list_runs(&command.instance_ref)
            .map_err(HostRuntimeError::Store)?
            .into_iter()
            .find(|run| run.run_id == run_id)
            .ok_or_else(|| HostRuntimeError::Incomplete(command.command_id.clone()))?;
        let status = turn_status(&run.status)?;
        let usage_ref =
            self.ensure_evidence(command, &run_id, "host.turn.usage", &run.metadata_json)?;
        let guarantee = json!({
            "protocol": HOST_PROTOCOL,
            "policy": command.policy,
            "package_version_ref": command.package_version_ref,
            "resources": command.resources,
            "images": command.input.images,
            "provider_binding_ref": command.provider_binding,
            "placement_ceiling_ref": command.placement_ceiling_ref,
            "guarantees": [
                "signed_policy_identity_verified",
                "package_ifc_checked_under_verified_envelope",
                "instance_package_policy_binding_verified",
                "resource_provider_placement_handles_governed",
                "tool_surface_pinned_to_package",
                "resource_and_secret_bodies_resolved_after_admission"
            ]
        })
        .to_string();
        let guarantee_report_ref =
            self.ensure_evidence(command, &run_id, "host.turn.guarantee", &guarantee)?;
        let events = self.project_events(command, &run_id)?;
        let output_handle =
            matches!(status, TurnStatus::Completed).then(|| format!("whip:run:{run_id}:output"));
        let marker_payload = json!({
            "command_id": command.command_id,
            "run_ref": command.run_ref,
            "status": status,
            "output_handle": output_handle,
            "usage_ref": usage_ref,
            "guarantee_report_ref": guarantee_report_ref,
        })
        .to_string();
        let marker = self
            .kernel
            .store()
            .append_event(NewEvent {
                instance_id: &command.instance_ref,
                event_type: "host.turn.receipt",
                payload_json: &marker_payload,
                source: "host-runtime",
                causation_id: Some(&run_id),
                correlation_id: Some(&command.command_id),
                idempotency_key: Some(&idempotency_key(&[
                    &command.instance_ref,
                    &command.command_id,
                    "host-turn-receipt",
                ])),
            })
            .map_err(HostRuntimeError::Store)?;
        let receipt = TurnReceipt {
            protocol: HOST_PROTOCOL.to_owned(),
            command_id: command.command_id.clone(),
            run_ref: command.run_ref.clone(),
            instance_ref: command.instance_ref.clone(),
            policy: command.policy.clone(),
            terminal_position: EventPosition {
                instance_ref: command.instance_ref.clone(),
                sequence: positive_sequence(marker.sequence)?,
            },
            status,
            output_handle,
            usage_ref,
            guarantee_report_ref,
            workspace_cut_ref: None,
        };
        receipt.validate_for(command)?;
        let output = self.project_turn_output(command, receipt.output_handle.clone())?;
        Ok(TurnExecution {
            events,
            receipt,
            output,
        })
    }

    fn ensure_evidence(
        &self,
        command: &StartTurnCommand,
        run_id: &str,
        kind: &str,
        metadata_json: &str,
    ) -> Result<String, HostRuntimeError> {
        let existing = self
            .kernel
            .store()
            .list_evidence_for_subject("run", run_id)
            .map_err(HostRuntimeError::Store)?;
        if let Some(evidence) = existing.iter().find(|evidence| {
            evidence.kind == kind && evidence.correlation_id.as_deref() == Some(&command.command_id)
        }) {
            return Ok(evidence.evidence_id.clone());
        }
        self.kernel
            .store()
            .record_evidence(EvidenceRecord {
                instance_id: &command.instance_ref,
                kind,
                subject_type: "run",
                subject_id: run_id,
                causation_id: Some(&command.command_id),
                correlation_id: Some(&command.command_id),
                summary: None,
                metadata_json,
            })
            .map_err(HostRuntimeError::Store)
    }

    fn project_events(
        &self,
        command: &StartTurnCommand,
        run_id: &str,
    ) -> Result<Vec<LabeledRuntimeEvent>, HostRuntimeError> {
        let evidence = self
            .kernel
            .store()
            .list_evidence_for_subject("run", run_id)
            .map_err(HostRuntimeError::Store)?;
        let mut projected = Vec::with_capacity(evidence.len());
        for item in evidence {
            let evidence_ref = format!("whip:evidence:{}", item.evidence_id);
            let payload = json!({
                "command_id": command.command_id,
                "kind": item.kind,
                "label_ref": self.label_ref(),
                "evidence_ref": evidence_ref,
            })
            .to_string();
            let event = self
                .kernel
                .store()
                .append_event(NewEvent {
                    instance_id: &command.instance_ref,
                    event_type: "host.turn.evidence",
                    payload_json: &payload,
                    source: "host-runtime",
                    causation_id: Some(run_id),
                    correlation_id: Some(&command.command_id),
                    idempotency_key: Some(&idempotency_key(&[
                        &command.instance_ref,
                        &command.command_id,
                        &item.evidence_id,
                        "host-evidence-projection",
                    ])),
                })
                .map_err(HostRuntimeError::Store)?;
            projected.push(LabeledRuntimeEvent {
                protocol: HOST_PROTOCOL.to_owned(),
                command_id: command.command_id.clone(),
                position: EventPosition {
                    instance_ref: command.instance_ref.clone(),
                    sequence: positive_sequence(event.sequence)?,
                },
                policy: command.policy.clone(),
                kind: item.kind,
                label_ref: self.label_ref(),
                evidence_ref,
                payload_ref: None,
            });
        }
        Ok(projected)
    }

    fn stored_execution(
        &self,
        command: &StartTurnCommand,
    ) -> Result<Option<TurnExecution>, HostRuntimeError> {
        let events = self
            .kernel
            .store()
            .list_events(&command.instance_ref)
            .map_err(HostRuntimeError::Store)?;
        let Some(marker) = events.iter().rev().find(|event| {
            event.event_type == "host.turn.receipt"
                && serde_json::from_str::<Value>(&event.payload_json)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("command_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .as_deref()
                    == Some(&command.command_id)
        }) else {
            return Ok(None);
        };
        let value: Value =
            serde_json::from_str(&marker.payload_json).map_err(HostRuntimeError::Json)?;
        let receipt = TurnReceipt {
            protocol: HOST_PROTOCOL.to_owned(),
            command_id: command.command_id.clone(),
            run_ref: required_string(&value, "run_ref")?,
            instance_ref: command.instance_ref.clone(),
            policy: command.policy.clone(),
            terminal_position: EventPosition {
                instance_ref: command.instance_ref.clone(),
                sequence: positive_sequence(marker.sequence)?,
            },
            status: serde_json::from_value(value["status"].clone())
                .map_err(HostRuntimeError::Json)?,
            output_handle: value
                .get("output_handle")
                .and_then(Value::as_str)
                .map(str::to_owned),
            usage_ref: required_string(&value, "usage_ref")?,
            guarantee_report_ref: required_string(&value, "guarantee_report_ref")?,
            workspace_cut_ref: None,
        };
        receipt.validate_for(command)?;
        let mut projected = Vec::new();
        for event in events {
            if event.event_type != "host.turn.evidence" {
                continue;
            }
            let payload: Value =
                serde_json::from_str(&event.payload_json).map_err(HostRuntimeError::Json)?;
            if payload.get("command_id").and_then(Value::as_str)
                != Some(command.command_id.as_str())
            {
                continue;
            }
            projected.push(LabeledRuntimeEvent {
                protocol: HOST_PROTOCOL.to_owned(),
                command_id: command.command_id.clone(),
                position: EventPosition {
                    instance_ref: command.instance_ref.clone(),
                    sequence: positive_sequence(event.sequence)?,
                },
                policy: command.policy.clone(),
                kind: required_string(&payload, "kind")?,
                label_ref: required_string(&payload, "label_ref")?,
                evidence_ref: required_string(&payload, "evidence_ref")?,
                payload_ref: None,
            });
        }
        let output = self.project_turn_output(command, receipt.output_handle.clone())?;
        Ok(Some(TurnExecution {
            events: projected,
            receipt,
            output,
        }))
    }

    fn project_turn_output(
        &self,
        command: &StartTurnCommand,
        output_handle: Option<String>,
    ) -> Result<Option<LabeledTurnOutput>, HostRuntimeError> {
        let events = self
            .kernel
            .store()
            .list_events(&command.instance_ref)
            .map_err(HostRuntimeError::Store)?;
        let Some(checkpoint) = events.iter().rev().find(|event| {
            event.event_type == "agent.turn.brokered.transcript"
                && serde_json::from_str::<Value>(&event.payload_json)
                    .ok()
                    .and_then(|payload| {
                        payload
                            .get("effect_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .as_deref()
                    == Some(&command.command_id)
        }) else {
            return Ok(None);
        };
        let value: Value =
            serde_json::from_str(&checkpoint.payload_json).map_err(HostRuntimeError::Json)?;
        let messages = whipplescript_kernel::harness_loop::chat_messages_from_json(
            value.get("messages").unwrap_or(&Value::Null),
        );
        let turn_start = messages
            .iter()
            .rposition(|message| matches!(message, ChatMessage::User { .. }))
            .map_or(0, |index| index + 1);
        let mut assistant_text = String::new();
        let mut tool_calls: Vec<ProjectedToolCall> = Vec::new();
        for message in &messages[turn_start..] {
            match message {
                ChatMessage::Assistant {
                    text,
                    tool_calls: calls,
                } => {
                    if calls.is_empty() {
                        assistant_text.clone_from(text);
                    } else {
                        tool_calls.extend(calls.iter().map(|call| ProjectedToolCall {
                            call_id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                            result: None,
                            ok: None,
                        }));
                    }
                }
                ChatMessage::ToolResults(results) => {
                    for result in results {
                        if let Some(projected) = tool_calls
                            .iter_mut()
                            .rev()
                            .find(|call| call.call_id == result.tool_call_id)
                        {
                            projected.result = Some(result.content.clone());
                            projected.ok = Some(!result.is_error);
                        }
                    }
                }
                ChatMessage::System(_) | ChatMessage::User { .. } => {}
            }
        }
        Ok(Some(LabeledTurnOutput {
            output_handle,
            label_ref: self.label_ref(),
            assistant_text,
            tool_calls,
        }))
    }

    fn require_policy(&self, policy: &PolicyEpochRef) -> Result<(), HostRuntimeError> {
        if policy == &self.policy {
            Ok(())
        } else {
            Err(HostRuntimeError::Protocol(ProtocolError::Mismatch(
                "runtime policy epoch",
            )))
        }
    }

    fn require_governed(&self, handle: &str) -> Result<(), HostRuntimeError> {
        if self.envelope.governs(handle) {
            Ok(())
        } else {
            Err(HostRuntimeError::UngovernedHandle(handle.to_owned()))
        }
    }

    fn check_package_ifc(&self, package: &ResolvedPackage) -> Result<(), HostRuntimeError> {
        let diagnostics = crate::ifc::check_with_envelope(&package.program, &self.envelope);
        if diagnostics.is_empty() {
            Ok(())
        } else {
            Err(HostRuntimeError::Ifc(
                diagnostics
                    .into_iter()
                    .map(|diagnostic| diagnostic.message)
                    .collect(),
            ))
        }
    }

    fn label_ref(&self) -> String {
        format!("whip:label:{}:turn-join", self.policy.envelope_hash)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct InstanceMetadata {
    protocol: String,
    package_version_ref: String,
    policy: PolicyEpochRef,
}

struct ResolverToolExecutor<'a, R: ResourceResolver + ?Sized> {
    offered: &'a [ToolSpec],
    admitted_resources: &'a [ResourceRef],
    resolver: &'a R,
}

impl<R: ResourceResolver + ?Sized> ToolExecutor for ResolverToolExecutor<'_, R> {
    fn execute(&self, call: &ToolCall) -> ToolOutcome {
        if !self.offered.iter().any(|tool| tool.name == call.name) {
            return ToolOutcome {
                status: ToolStatus::Error,
                content: "tool is not declared by the pinned package".to_owned(),
            };
        }
        match self.resolver.execute_tool(self.admitted_resources, call) {
            Ok(content) => ToolOutcome {
                status: ToolStatus::Ok,
                content,
            },
            Err(message) => ToolOutcome {
                status: ToolStatus::Error,
                content: message,
            },
        }
    }
}

struct NativeHttpDriver {
    agent: ureq::Agent,
}

impl NativeHttpDriver {
    fn new(timeout: Duration) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(timeout)
                // A governed provider binding names the exact egress endpoint.
                // Following a provider-controlled redirect would widen that
                // capability (and could replay prompt content to another host),
                // so redirects fail closed and must be resolved by the host.
                .redirects(0)
                .user_agent("whipplescript-host-runtime")
                .build(),
        }
    }
}

impl HostDriver for NativeHttpDriver {
    fn fulfill(&self, request: &IoRequest) -> IoResult {
        let IoRequest::Http(request) = request;
        let mut builder = self.agent.post(&request.url);
        for (name, value) in &request.headers {
            builder = builder.set(name, value);
        }
        let response = match builder.send_json(request.body.clone()) {
            Ok(response) | Err(ureq::Error::Status(_, response)) => response,
            Err(ureq::Error::Transport(error)) => {
                let message = error.to_string();
                let error = if message.to_ascii_lowercase().contains("timeout") {
                    TransportError::Timeout
                } else {
                    TransportError::Transport(message)
                };
                return IoResult::Http(Err(error));
            }
        };
        let expects_sse = request.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("accept") && value == "text/event-stream"
        });
        let status = response.status();
        let body = if expects_sse {
            assemble_responses_sse(&response.into_string().unwrap_or_default())
        } else {
            response.into_json::<Value>().unwrap_or(Value::Null)
        };
        IoResult::Http(Ok(HttpResponse { status, body }))
    }
}

fn assemble_responses_sse(raw: &str) -> Value {
    let mut completed: Option<Value> = None;
    let mut deltas = String::new();
    for line in raw.lines() {
        let Some(payload) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("response.completed") => completed = event.get("response").cloned(),
            Some("response.output_text.delta") => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    deltas.push_str(delta);
                }
            }
            _ => {}
        }
    }
    let mut response = completed.unwrap_or_else(|| json!({}));
    if !deltas.is_empty() {
        response["output_text"] = Value::String(deltas);
    }
    response
}

#[derive(Debug)]
pub enum HostRuntimeError {
    Protocol(ProtocolError),
    PolicyRejected(String),
    UngovernedHandle(String),
    Ifc(Vec<String>),
    UnknownInstance(String),
    Incomplete(String),
    Resolver(String),
    Store(StoreError),
    Json(serde_json::Error),
}

impl fmt::Display for HostRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => error.fmt(formatter),
            Self::PolicyRejected(message) => write!(formatter, "policy rejected: {message}"),
            Self::UngovernedHandle(handle) => {
                write!(formatter, "host handle is not governed: {handle}")
            }
            Self::Ifc(diagnostics) => write!(
                formatter,
                "package violates the admitted information-flow policy: {}",
                diagnostics.join("; ")
            ),
            Self::UnknownInstance(instance) => write!(formatter, "unknown instance: {instance}"),
            Self::Incomplete(command) => write!(formatter, "turn is not terminal: {command}"),
            Self::Resolver(message) => write!(formatter, "host resolver refused: {message}"),
            Self::Store(error) => write!(formatter, "runtime store error: {error:?}"),
            Self::Json(error) => write!(formatter, "runtime JSON error: {error}"),
        }
    }
}

impl std::error::Error for HostRuntimeError {}

impl From<ProtocolError> for HostRuntimeError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

fn validate_package(package: &ResolvedPackage, expected_ref: &str) -> Result<(), HostRuntimeError> {
    if package.version_ref != expected_ref {
        return Err(HostRuntimeError::Protocol(ProtocolError::Mismatch(
            "resolved package ref",
        )));
    }
    if package.source_hash.trim().is_empty()
        || package.ir_hash.trim().is_empty()
        || package.agent.trim().is_empty()
        || package.max_steps == 0
        || !package
            .program
            .agents
            .iter()
            .any(|agent| agent.name == package.agent)
    {
        return Err(HostRuntimeError::Resolver(
            "resolved package is incomplete".to_owned(),
        ));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn positive_sequence(sequence: i64) -> Result<u64, HostRuntimeError> {
    u64::try_from(sequence)
        .ok()
        .filter(|sequence| *sequence > 0)
        .ok_or(HostRuntimeError::Protocol(ProtocolError::Invalid(
            "runtime event sequence must be positive",
        )))
}

fn required_string(value: &Value, key: &'static str) -> Result<String, HostRuntimeError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or(HostRuntimeError::Protocol(ProtocolError::Invalid(key)))
}

fn is_terminal_effect(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "timed_out" | "cancelled")
}

fn turn_status(status: &str) -> Result<TurnStatus, HostRuntimeError> {
    match status {
        "completed" => Ok(TurnStatus::Completed),
        "failed" => Ok(TurnStatus::Failed),
        "timed_out" => Ok(TurnStatus::TimedOut),
        "cancelled" => Ok(TurnStatus::Cancelled),
        _ => Err(HostRuntimeError::Incomplete(status.to_owned())),
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        encoded.push(ALPHABET[(a >> 2) as usize] as char);
        encoded.push(ALPHABET[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[(((b & 0x0f) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(c & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::gov::SignedEnvelope;
    use crate::host_protocol::TurnInput;

    struct Packages;

    impl PackageResolver for Packages {
        fn resolve_package(&self, version_ref: &str) -> Result<ResolvedPackage, String> {
            ResolvedPackage::compile(
                version_ref,
                r#"
file store project {
  root "."
  allow read ["**"]
  allow write ["**"]
}

workflow HostChat {
  agent assistant {
    provider owned
    profile "repo-writer"
    capacity 1
  }

  rule converse
    when started
  => {
    tell assistant
      with access to project {
        read ["**"]
        write ["**"]
      }
      "host turn"
  }
}
"#,
                Some("HostChat"),
                "assistant",
                "Help through the governed resource tools.",
                vec![ToolSpec {
                    name: "read".to_owned(),
                    description: "Read an admitted resource.".to_owned(),
                    input_schema: json!({
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"],
                        "additionalProperties": false
                    }),
                }],
                4,
            )
        }
    }

    struct UnsafePackages;

    impl PackageResolver for UnsafePackages {
        fn resolve_package(&self, version_ref: &str) -> Result<ResolvedPackage, String> {
            ResolvedPackage::compile(
                version_ref,
                r#"
file store project {
  root "."
  allow read ["**"]
}

workflow UnsafeHostChat {
  agent assistant {
    provider fixture
    profile "repo-reader"
    capacity 1
  }

  rule leak
    when started
  => {
    tell assistant
      with access to project {
        read ["**"]
      }
      "leak the project"
  }
}
"#,
                Some("UnsafeHostChat"),
                "assistant",
                "unsafe",
                Vec::new(),
                1,
            )
        }
    }

    struct Secrets {
        calls: Cell<usize>,
    }

    impl SecretResolver for Secrets {
        fn resolve_provider(
            &self,
            binding: &ProviderBindingRef,
            placement_ceiling_ref: &str,
        ) -> Result<ResolvedProviderBinding, String> {
            assert_eq!(binding.binding_id, "model");
            assert_eq!(placement_ceiling_ref, "local");
            self.calls.set(self.calls.get() + 1);
            Ok(ResolvedProviderBinding::new(
                ModelProvider::OpenAi,
                "secret-that-must-not-be-persisted",
                "gpt-test",
                "https://provider.invalid",
                256,
                Duration::from_secs(1),
            ))
        }
    }

    struct Resources {
        calls: Cell<usize>,
    }

    impl ResourceResolver for Resources {
        fn resolve_image(&self, _image: &ResourceRef) -> Result<ResolvedImage, String> {
            Err("no images in this test".to_owned())
        }

        fn execute_tool(
            &self,
            admitted_resources: &[ResourceRef],
            call: &ToolCall,
        ) -> Result<String, String> {
            assert_eq!(call.name, "read");
            assert_eq!(admitted_resources.len(), 1);
            assert_eq!(admitted_resources[0].handle, "project");
            self.calls.set(self.calls.get() + 1);
            Ok("governed file body".to_owned())
        }
    }

    struct ScriptedDriver {
        replies: RefCell<VecDeque<Value>>,
        requests: RefCell<Vec<Value>>,
    }

    impl ScriptedDriver {
        fn new(replies: Vec<Value>) -> Self {
            Self {
                replies: RefCell::new(replies.into()),
                requests: RefCell::new(Vec::new()),
            }
        }
    }

    impl HostDriver for ScriptedDriver {
        fn fulfill(&self, request: &IoRequest) -> IoResult {
            let IoRequest::Http(request) = request;
            self.requests.borrow_mut().push(request.body.clone());
            IoResult::Http(Ok(HttpResponse {
                status: 200,
                body: self
                    .replies
                    .borrow_mut()
                    .pop_front()
                    .expect("scripted reply"),
            }))
        }
    }

    struct CancellingDriver {
        handle: HostCancellationHandle,
        fired: Cell<bool>,
    }

    impl HostDriver for CancellingDriver {
        fn fulfill(&self, _request: &IoRequest) -> IoResult {
            assert!(
                !self.fired.replace(true),
                "cancelled before a second request"
            );
            self.handle.request().expect("cancel request records");
            IoResult::Http(Ok(HttpResponse {
                status: 200,
                body: json!({
                    "output": [{
                        "type": "function_call",
                        "call_id": "call-before-cancel",
                        "name": "read",
                        "arguments": "{\"path\":\"README.md\"}"
                    }],
                    "usage": { "input_tokens": 10, "output_tokens": 2 }
                }),
            }))
        }
    }

    fn signed_policy() -> String {
        SignedEnvelope::sign_for_test(
            "grant file_store project -> file:/workspace readable by Operator\n\
             grant provider model -> provider:openai readable by Operator\n\
             grant provider owned -> provider:owned readable by Operator\n\
             grant placement local -> placement:local readable by Operator\n",
            "gaugedesk-admin",
        )
        .to_json()
    }

    fn temp_store() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "whip-host-runtime-{}-{nonce}.sqlite",
            std::process::id()
        ))
    }

    fn turn(instance_ref: &str, policy: &PolicyEpochRef, number: usize) -> StartTurnCommand {
        StartTurnCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            command_id: format!("command-{number}"),
            run_ref: format!("gaugedesk:run:{number}"),
            instance_ref: instance_ref.to_owned(),
            package_version_ref: "package:v1".to_owned(),
            policy: policy.clone(),
            input: TurnInput {
                text: format!("turn {number}"),
                images: Vec::new(),
            },
            resources: vec![ResourceRef {
                handle: "project".to_owned(),
                kind: "file_store".to_owned(),
                selector: None,
            }],
            provider_binding: ProviderBindingRef {
                binding_id: "model".to_owned(),
            },
            placement_ceiling_ref: "local".to_owned(),
        }
    }

    #[test]
    fn persistent_owned_turn_reopens_with_transcript_and_never_persists_secret() {
        let path = temp_store();
        let policy_text = signed_policy();
        let mut runtime = GovernedHostRuntime::open(&path, 7, &policy_text).expect("runtime");
        let open = OpenInstanceCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: "open-chat".to_owned(),
            package_version_ref: "package:v1".to_owned(),
            policy: runtime.policy_ref().clone(),
        };
        let instance = runtime.open_instance(&open, &Packages).expect("instance");
        instance.validate_for(&open).expect("opened binding");

        let secrets = Secrets {
            calls: Cell::new(0),
        };
        let resources = Resources {
            calls: Cell::new(0),
        };
        let first_driver = ScriptedDriver::new(vec![
            json!({
                "output": [{
                    "type": "function_call",
                    "call_id": "call-1",
                    "name": "read",
                    "arguments": "{\"path\":\"README.md\"}"
                }],
                "usage": { "input_tokens": 10, "output_tokens": 2 }
            }),
            json!({
                "output_text": "first answer",
                "usage": { "input_tokens": 14, "output_tokens": 3 }
            }),
        ]);
        let first = runtime
            .run_turn_with_driver(
                &turn(&instance.instance_ref, &open.policy, 1),
                &Packages,
                &secrets,
                &resources,
                &first_driver,
            )
            .expect("first turn");
        assert_eq!(first.receipt.status, TurnStatus::Completed);
        assert!(!first.events.is_empty());
        let first_output = first.output.expect("labeled output projection");
        assert_eq!(first_output.output_handle, first.receipt.output_handle);
        assert_eq!(first_output.assistant_text, "first answer");
        assert_eq!(first_output.tool_calls.len(), 1);
        assert_eq!(first_output.tool_calls[0].name, "read");
        assert_eq!(
            first_output.tool_calls[0].arguments,
            json!({ "path": "README.md" })
        );
        assert_eq!(
            first_output.tool_calls[0].result.as_deref(),
            Some("governed file body")
        );
        assert_eq!(first_output.tool_calls[0].ok, Some(true));
        assert_eq!(resources.calls.get(), 1);
        drop(runtime);

        let mut reopened = GovernedHostRuntime::open(&path, 7, &policy_text).expect("reopen");
        let replayed = reopened
            .open_instance(&open, &Packages)
            .expect("open command replays");
        assert_eq!(replayed.instance_ref, instance.instance_ref);
        assert_eq!(replayed.opened_at, instance.opened_at);
        let second_driver = ScriptedDriver::new(vec![json!({
            "output_text": "second answer",
            "usage": { "input_tokens": 20, "output_tokens": 3 }
        })]);
        let second = reopened
            .run_turn_with_driver(
                &turn(&instance.instance_ref, &open.policy, 2),
                &Packages,
                &secrets,
                &resources,
                &second_driver,
            )
            .expect("second turn");
        assert_eq!(second.receipt.status, TurnStatus::Completed);
        assert_eq!(
            second
                .output
                .as_ref()
                .map(|output| output.assistant_text.as_str()),
            Some("second answer")
        );
        let request = second_driver.requests.borrow();
        let serialized = request.first().expect("request").to_string();
        assert!(serialized.contains("first answer"));
        assert!(serialized.contains("turn 2"));
        drop(reopened);

        let bytes = fs::read(&path).expect("store bytes");
        assert!(!String::from_utf8_lossy(&bytes).contains("secret-that-must-not-be-persisted"));
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn governed_instance_fork_seeds_a_distinct_continuing_thread() {
        let source_path = temp_store();
        let target_path = temp_store();
        let policy_text = signed_policy();
        let mut source =
            GovernedHostRuntime::open(&source_path, 9, &policy_text).expect("source runtime");
        let source_open = OpenInstanceCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: "open-source-chat".to_owned(),
            package_version_ref: "package:v1".to_owned(),
            policy: source.policy_ref().clone(),
        };
        let source_instance = source
            .open_instance(&source_open, &Packages)
            .expect("source instance");
        source
            .run_turn_with_driver(
                &turn(&source_instance.instance_ref, &source_open.policy, 1),
                &Packages,
                &Secrets {
                    calls: Cell::new(0),
                },
                &Resources {
                    calls: Cell::new(0),
                },
                &ScriptedDriver::new(vec![json!({
                    "output_text": "source answer",
                    "usage": { "input_tokens": 10, "output_tokens": 3 }
                })]),
            )
            .expect("source turn");
        let source_position = source
            .current_position(&source_instance.instance_ref)
            .expect("source position");

        let mut target =
            GovernedHostRuntime::open(&target_path, 9, &policy_text).expect("target runtime");
        let fork = ForkInstanceCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: "fork-source-into-target".to_owned(),
            source: source_position,
            target_request_id: "open-target-chat".to_owned(),
            package_version_ref: "package:v1".to_owned(),
            policy: target.policy_ref().clone(),
        };
        let forked = target
            .fork_instance_from(&source, &fork, &Packages)
            .expect("fork succeeds");
        forked.validate_for(&fork).expect("fork binding");
        assert_ne!(forked.target.instance_ref, source_instance.instance_ref);
        let replayed = target
            .fork_instance_from(&source, &fork, &Packages)
            .expect("fork replays");
        assert_eq!(replayed, forked);

        let driver = ScriptedDriver::new(vec![json!({
            "output_text": "target answer",
            "usage": { "input_tokens": 20, "output_tokens": 3 }
        })]);
        target
            .run_turn_with_driver(
                &turn(&forked.target.instance_ref, &fork.policy, 2),
                &Packages,
                &Secrets {
                    calls: Cell::new(0),
                },
                &Resources {
                    calls: Cell::new(0),
                },
                &driver,
            )
            .expect("target turn");
        let serialized = driver
            .requests
            .borrow()
            .first()
            .expect("target request")
            .to_string();
        assert!(serialized.contains("source answer"));
        assert!(serialized.contains("turn 2"));

        drop(target);
        drop(source);
        for path in [&source_path, &target_path] {
            let _ = fs::remove_file(path);
            let _ = fs::remove_file(path.with_extension("sqlite-wal"));
            let _ = fs::remove_file(path.with_extension("sqlite-shm"));
        }
    }

    #[test]
    fn ungoverned_resource_is_rejected_before_secret_resolution() {
        let path = temp_store();
        let policy_text = signed_policy();
        let mut runtime = GovernedHostRuntime::open(&path, 3, &policy_text).expect("runtime");
        let open = OpenInstanceCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: "open-chat".to_owned(),
            package_version_ref: "package:v1".to_owned(),
            policy: runtime.policy_ref().clone(),
        };
        let instance = runtime.open_instance(&open, &Packages).expect("instance");
        let mut command = turn(&instance.instance_ref, &open.policy, 1);
        command.resources[0].handle = "unlisted".to_owned();
        let secrets = Secrets {
            calls: Cell::new(0),
        };
        let resources = Resources {
            calls: Cell::new(0),
        };
        let driver = ScriptedDriver::new(Vec::new());
        let error = runtime
            .run_turn_with_driver(&command, &Packages, &secrets, &resources, &driver)
            .expect_err("ungoverned resource");
        assert!(matches!(error, HostRuntimeError::UngovernedHandle(_)));
        assert_eq!(secrets.calls.get(), 0);
        drop(runtime);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn out_of_band_handle_requests_cooperative_cancellation() {
        let path = temp_store();
        let policy_text = signed_policy();
        let mut runtime = GovernedHostRuntime::open(&path, 8, &policy_text).expect("runtime");
        let open = OpenInstanceCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: "open-cancel-chat".to_owned(),
            package_version_ref: "package:v1".to_owned(),
            policy: runtime.policy_ref().clone(),
        };
        let instance = runtime.open_instance(&open, &Packages).expect("instance");
        let command = turn(&instance.instance_ref, &open.policy, 1);
        let driver = CancellingDriver {
            handle: runtime.cancellation_handle(&command.instance_ref, &command.command_id),
            fired: Cell::new(false),
        };
        let execution = runtime
            .run_turn_with_driver(
                &command,
                &Packages,
                &Secrets {
                    calls: Cell::new(0),
                },
                &Resources {
                    calls: Cell::new(0),
                },
                &driver,
            )
            .expect("cancelled execution settles");
        assert_eq!(execution.receipt.status, TurnStatus::Cancelled);
        assert!(execution.receipt.output_handle.is_none());
        assert!(driver.fired.get());
        drop(runtime);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn package_ifc_violation_is_rejected_before_an_instance_opens() {
        let path = temp_store();
        let policy_text = signed_policy();
        let mut runtime = GovernedHostRuntime::open(&path, 4, &policy_text).expect("runtime");
        let open = OpenInstanceCommand {
            protocol: HOST_PROTOCOL.to_owned(),
            request_id: "open-unsafe-chat".to_owned(),
            package_version_ref: "package:unsafe".to_owned(),
            policy: runtime.policy_ref().clone(),
        };
        let error = runtime
            .open_instance(&open, &UnsafePackages)
            .expect_err("IFC violation");
        assert!(matches!(error, HostRuntimeError::Ifc(_)));
        drop(runtime);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn native_workspace_tools_are_confined_and_honor_read_only_subtrees() {
        let root = std::env::temp_dir().join(format!(
            "whip-native-workspace-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(root.join(".pi")).expect("dirs");
        fs::write(root.join("note.txt"), "alpha\nbeta\n").expect("note");
        fs::write(root.join(".pi/SYSTEM.md"), "protected").expect("method");
        let resolver = NativeWorkspaceResolver::new(&root)
            .expect("resolver")
            .read_only([PathBuf::from(".pi")])
            .expect("read-only path");
        let resources = [ResourceRef {
            handle: "project".to_owned(),
            kind: "file_store".to_owned(),
            selector: None,
        }];

        let read = resolver
            .execute_tool(
                &resources,
                &ToolCall {
                    id: "read-1".to_owned(),
                    name: "read".to_owned(),
                    arguments: json!({ "path": "note.txt" }),
                },
            )
            .expect("read");
        assert!(read.contains("1: alpha"));
        resolver
            .execute_tool(
                &resources,
                &ToolCall {
                    id: "edit-1".to_owned(),
                    name: "edit".to_owned(),
                    arguments: json!({
                        "path": "note.txt",
                        "edits": [{ "oldText": "beta", "newText": "gamma" }]
                    }),
                },
            )
            .expect("edit");
        assert_eq!(
            fs::read_to_string(root.join("note.txt")).expect("edited note"),
            "alpha\ngamma\n"
        );

        for path in ["../outside", ".pi/SYSTEM.md"] {
            assert!(resolver
                .execute_tool(
                    &resources,
                    &ToolCall {
                        id: "write-denied".to_owned(),
                        name: "write".to_owned(),
                        arguments: json!({ "path": path, "content": "tampered" }),
                    },
                )
                .is_err());
        }
        assert_eq!(
            fs::read_to_string(root.join(".pi/SYSTEM.md")).expect("protected method"),
            "protected"
        );
        assert!(native_workspace_tool_specs(false)
            .iter()
            .all(|tool| tool.name != "write" && tool.name != "edit"));
        assert!(native_workspace_tool_specs(true)
            .iter()
            .any(|tool| tool.name == "write"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_fingerprint_covers_behavior_outside_workflow_source() {
        let source = r#"
workflow Fingerprint {
  agent assistant {
    provider owned
    profile "repo-writer"
    capacity 1
  }
  rule converse when started => { tell assistant "hello" }
}
"#;
        let compile = |prompt: &str, writable: bool| {
            ResolvedPackage::compile(
                "package:fingerprint",
                source,
                Some("Fingerprint"),
                "assistant",
                prompt,
                native_workspace_tool_specs(writable),
                4,
            )
            .expect("package compiles")
        };
        let original = compile("first prompt", false);
        assert_ne!(
            original.source_hash,
            compile("second prompt", false).source_hash
        );
        assert_ne!(
            original.source_hash,
            compile("first prompt", true).source_hash
        );
        let more_steps = ResolvedPackage::compile(
            "package:fingerprint",
            source,
            Some("Fingerprint"),
            "assistant",
            "first prompt",
            native_workspace_tool_specs(false),
            5,
        )
        .expect("package compiles");
        assert_ne!(original.source_hash, more_steps.source_hash);
    }

    #[test]
    fn codex_sse_assembly_preserves_calls_and_text_deltas() {
        let raw = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"done\"}\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"function_call\",\"call_id\":\"c1\",\"name\":\"read\",\"arguments\":\"{}\"}],\"usage\":{\"input_tokens\":3}}}\n",
            "data: [DONE]\n",
        );
        let response = assemble_responses_sse(raw);
        assert_eq!(response["output_text"], "done");
        assert_eq!(response["output"][0]["call_id"], "c1");
        assert_eq!(response["usage"]["input_tokens"], 3);
    }
}
