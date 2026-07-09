//! Persistent native host facade for governed WhippleScript turns.
//!
//! The facade owns policy admission, instance identity, the brokered model/tool
//! loop, transcript persistence, and evidence projection. Embedding products
//! provide only opaque-reference resolvers. Secrets and resource bodies are
//! resolved after admission and never enter the host command or receipt.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use whipplescript_kernel::coerce_native::CoerceProvider;
use whipplescript_kernel::harness_loop::{
    BrokeredTurnInput, ImageBlock, NoopCompactor, ToolCall, ToolExecutor, ToolOutcome, ToolSpec,
    ToolStatus,
};
use whipplescript_kernel::harness_model::MessagesApiClient;
use whipplescript_kernel::sansio::{HostDriver, HttpResponse, IoRequest, IoResult, TransportError};
use whipplescript_kernel::{
    idempotency_key, BrokeredTurnContext, ProgramVersionInput, RuntimeKernel,
};
use whipplescript_store::{
    EvidenceRecord, NewEffect, NewEvent, RuleCommit, SqliteStore, StoreError,
};

use crate::host_protocol::{
    EventPosition, LabeledRuntimeEvent, OpenInstanceCommand, OpenedInstance, PolicyEpochRef,
    ProtocolError, ProviderBindingRef, ResourceRef, StartTurnCommand, TurnReceipt, TurnStatus,
    HOST_PROTOCOL,
};
use crate::ifc::VerifiedEnvelope;

/// A package version resolved from WhippleScript's package store.
///
/// Tool schemas come from the pinned package. The embedding host cannot add a
/// tool at turn time; it only implements the resource operations behind tools
/// the package already declares.
#[derive(Clone, Debug)]
pub struct ResolvedPackage {
    pub version_ref: String,
    pub source_hash: String,
    pub ir_hash: String,
    pub agent: String,
    pub system_prompt: String,
    pub tools: Vec<ToolSpec>,
    pub max_steps: usize,
}

/// Resolve an immutable WhippleScript package version by opaque reference.
pub trait PackageResolver {
    fn resolve_package(&self, version_ref: &str) -> Result<ResolvedPackage, String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelProvider {
    OpenAi,
    Anthropic,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnExecution {
    pub events: Vec<LabeledRuntimeEvent>,
    pub receipt: TurnReceipt,
}

/// A persistent, policy-bound native WhippleScript runtime.
pub struct GovernedHostRuntime {
    kernel: RuntimeKernel<SqliteStore>,
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
        let policy = PolicyEpochRef::from_verified(epoch, &envelope)?;
        let store = SqliteStore::open(store_path).map_err(HostRuntimeError::Store)?;
        Ok(Self {
            kernel: RuntimeKernel::new(store),
            policy,
            envelope,
        })
    }

    pub fn policy_ref(&self) -> &PolicyEpochRef {
        &self.policy
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
        let provider = match binding.provider {
            ModelProvider::OpenAi => CoerceProvider::OpenAi,
            ModelProvider::Anthropic => CoerceProvider::Anthropic,
        };
        let client = MessagesApiClient::new(
            provider,
            binding.api_key,
            binding.model,
            binding.base_url,
            binding.max_tokens,
            Some(command.command_id.clone()),
        );
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
        let instance = self
            .kernel
            .store()
            .get_instance(&command.instance_ref)
            .map_err(HostRuntimeError::Store)?
            .ok_or_else(|| HostRuntimeError::UnknownInstance(command.instance_ref.clone()))?;
        let metadata: InstanceMetadata =
            serde_json::from_str(&instance.input_json).map_err(HostRuntimeError::Json)?;
        if metadata.protocol != HOST_PROTOCOL
            || metadata.package_version_ref != command.package_version_ref
            || metadata.policy != command.policy
        {
            return Err(HostRuntimeError::Protocol(ProtocolError::Mismatch(
                "instance package/policy binding",
            )));
        }
        let package = packages
            .resolve_package(&command.package_version_ref)
            .map_err(HostRuntimeError::Resolver)?;
        validate_package(&package, &command.package_version_ref)?;
        let version = self
            .kernel
            .store()
            .get_program_version(&instance.version_id)
            .map_err(HostRuntimeError::Store)?
            .ok_or_else(|| HostRuntimeError::UnknownInstance(command.instance_ref.clone()))?;
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
        Ok(TurnExecution { events, receipt })
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
        Ok(Some(TurnExecution {
            events: projected,
            receipt,
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
        IoResult::Http(Ok(HttpResponse {
            status: response.status(),
            body: response.into_json::<Value>().unwrap_or(Value::Null),
        }))
    }
}

#[derive(Debug)]
pub enum HostRuntimeError {
    Protocol(ProtocolError),
    PolicyRejected(String),
    UngovernedHandle(String),
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
    {
        return Err(HostRuntimeError::Resolver(
            "resolved package is incomplete".to_owned(),
        ));
    }
    Ok(())
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
            Ok(ResolvedPackage {
                version_ref: version_ref.to_owned(),
                source_hash: "source-v1".to_owned(),
                ir_hash: "ir-v1".to_owned(),
                agent: "assistant".to_owned(),
                system_prompt: "Help through the governed resource tools.".to_owned(),
                tools: vec![ToolSpec {
                    name: "read".to_owned(),
                    description: "Read an admitted resource.".to_owned(),
                    input_schema: json!({
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"],
                        "additionalProperties": false
                    }),
                }],
                max_steps: 4,
            })
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

    fn signed_policy() -> String {
        SignedEnvelope::sign_for_test(
            "grant file_store project -> file:/workspace readable by Operator\n\
             grant provider model -> provider:openai readable by Operator\n\
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
        assert_eq!(resources.calls.get(), 1);
        drop(runtime);

        let mut reopened = GovernedHostRuntime::open(&path, 7, &policy_text).expect("reopen");
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
}
