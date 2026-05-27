//! Adapter boundaries for workflow effects.
//!
//! Adapters are trusted runtime code. Workflow files can request typed effects,
//! but they cannot execute adapter implementation code directly.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPolicyDocument {
    #[serde(default = "default_policy_mode")]
    pub mode: armature_workflow::policy::PolicyMode,
    #[serde(default)]
    pub allowed_capabilities: Vec<String>,
    #[serde(default)]
    pub denied_capabilities: Vec<String>,
    #[serde(default)]
    pub allow_baml_network: Option<bool>,
    #[serde(default)]
    pub allowed_baml_urls: Vec<String>,
    #[serde(default)]
    pub allow_managed_baml_server: Option<bool>,
    #[serde(default)]
    pub allowed_models: Vec<String>,
    #[serde(default)]
    pub allowed_env_vars: Vec<String>,
    #[serde(default)]
    pub store_baml_raw_responses: Option<bool>,
}

fn default_policy_mode() -> armature_workflow::policy::PolicyMode {
    armature_workflow::policy::PolicyMode::Local
}

impl Default for CapabilityPolicyDocument {
    fn default() -> Self {
        Self {
            mode: default_policy_mode(),
            allowed_capabilities: Vec::new(),
            denied_capabilities: Vec::new(),
            allow_baml_network: None,
            allowed_baml_urls: Vec::new(),
            allow_managed_baml_server: None,
            allowed_models: Vec::new(),
            allowed_env_vars: Vec::new(),
            store_baml_raw_responses: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub types: BTreeMap<String, armature_workflow::schema::Schema>,
    #[serde(default)]
    pub effects: BTreeMap<String, AdapterEffect>,
    #[serde(default)]
    pub events: BTreeMap<String, armature_workflow::schema::Schema>,
}

impl AdapterManifest {
    pub fn diagnostics(&self) -> Vec<armature_workflow::Diagnostic> {
        validate_adapter_manifests(std::slice::from_ref(self))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterEffect {
    pub category: armature_engine::effects::EffectCategory,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    pub input: armature_workflow::schema::Schema,
    pub output: armature_workflow::schema::Schema,
    pub idempotent: bool,
    #[serde(default)]
    pub failure_categories: Vec<String>,
    pub model: Option<AdapterModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdapterModel {
    Deterministic,
    NondeterministicOutcome { values: Vec<String> },
    Opaque,
}

pub fn json_plan_adapter_manifest() -> AdapterManifest {
    let mut effects = BTreeMap::new();
    effects.insert(
        "plan.snapshot".to_string(),
        AdapterEffect {
            category: armature_engine::effects::EffectCategory::SyncValue,
            required_capabilities: vec!["resource.plan.read".to_string()],
            input: armature_workflow::schema::Schema::Json,
            output: armature_workflow::schema::Schema::String,
            idempotent: true,
            failure_categories: vec!["resource_unavailable".to_string()],
            model: Some(AdapterModel::Deterministic),
        },
    );
    effects.insert(
        "plan.unfinishedItems".to_string(),
        AdapterEffect {
            category: armature_engine::effects::EffectCategory::SyncValue,
            required_capabilities: vec!["resource.plan.read".to_string()],
            input: armature_workflow::schema::Schema::Json,
            output: armature_workflow::schema::Schema::Int,
            idempotent: true,
            failure_categories: vec!["resource_unavailable".to_string()],
            model: Some(AdapterModel::Deterministic),
        },
    );
    effects.insert(
        "plan.nextReadyItem".to_string(),
        AdapterEffect {
            category: armature_engine::effects::EffectCategory::SyncValue,
            required_capabilities: vec!["resource.plan.read".to_string()],
            input: armature_workflow::schema::Schema::Json,
            output: armature_workflow::schema::Schema::Json,
            idempotent: true,
            failure_categories: vec!["resource_unavailable".to_string()],
            model: Some(AdapterModel::Deterministic),
        },
    );
    for effect_name in [
        "plan.markReadyForQuality",
        "plan.markBlocked",
        "plan.markDone",
    ] {
        effects.insert(
            effect_name.to_string(),
            AdapterEffect {
                category: armature_engine::effects::EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.write".to_string()],
                input: armature_workflow::schema::Schema::Json,
                output: armature_workflow::schema::Schema::Json,
                idempotent: true,
                failure_categories: vec!["resource_unavailable".to_string()],
                model: Some(AdapterModel::Opaque),
            },
        );
    }

    AdapterManifest {
        name: "json-plan-file".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        types: BTreeMap::new(),
        effects,
        events: BTreeMap::new(),
    }
}

pub fn json_human_review_response_event_schema() -> armature_workflow::schema::Schema {
    armature_workflow::schema::Schema::Record {
        fields: vec![
            armature_workflow::schema::Field {
                name: "reviewId".to_string(),
                schema: armature_workflow::schema::Schema::String,
            },
            armature_workflow::schema::Field {
                name: "decision".to_string(),
                schema: armature_workflow::schema::Schema::String,
            },
            armature_workflow::schema::Field {
                name: "response".to_string(),
                schema: armature_workflow::schema::Schema::Optional {
                    inner: Box::new(armature_workflow::schema::Schema::String),
                },
            },
        ],
    }
}

pub fn json_human_review_response_event_manifest() -> AdapterManifest {
    let mut events = BTreeMap::new();
    events.insert(
        "humanReview.responded".to_string(),
        json_human_review_response_event_schema(),
    );

    AdapterManifest {
        name: "json-human-review-response-events".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        types: BTreeMap::new(),
        effects: BTreeMap::new(),
        events,
    }
}

pub fn json_human_review_adapter_manifest() -> AdapterManifest {
    let mut effects = BTreeMap::new();
    effects.insert(
        "askHuman".to_string(),
        AdapterEffect {
            category: armature_engine::effects::EffectCategory::HumanObligation,
            required_capabilities: vec!["askHuman".to_string()],
            input: armature_workflow::schema::Schema::Json,
            output: armature_workflow::schema::Schema::Json,
            idempotent: true,
            failure_categories: vec!["review_unavailable".to_string()],
            model: Some(AdapterModel::Opaque),
        },
    );
    let events = json_human_review_response_event_manifest().events;

    AdapterManifest {
        name: "json-human-review-file".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        types: BTreeMap::new(),
        effects,
        events,
    }
}

pub fn json_agent_adapter_manifest() -> AdapterManifest {
    let mut effects = BTreeMap::new();
    effects.insert(
        "start".to_string(),
        AdapterEffect {
            category: armature_engine::effects::EffectCategory::AsyncInvocation,
            required_capabilities: vec!["adapter.agent.start".to_string()],
            input: armature_workflow::schema::Schema::Json,
            output: armature_workflow::schema::Schema::Json,
            idempotent: true,
            failure_categories: vec!["adapter_failure".to_string(), "timeout".to_string()],
            model: Some(AdapterModel::NondeterministicOutcome {
                values: vec![
                    "accepted".to_string(),
                    "rejected".to_string(),
                    "failed".to_string(),
                ],
            }),
        },
    );
    effects.insert(
        "send".to_string(),
        AdapterEffect {
            category: armature_engine::effects::EffectCategory::Message,
            required_capabilities: vec!["message_agents".to_string()],
            input: armature_workflow::schema::Schema::Json,
            output: armature_workflow::schema::Schema::Json,
            idempotent: true,
            failure_categories: vec!["delivery_failed".to_string()],
            model: Some(AdapterModel::Opaque),
        },
    );

    AdapterManifest {
        name: "json-agent-file".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        types: BTreeMap::new(),
        effects,
        events: BTreeMap::new(),
    }
}

pub fn json_agent_finished_event_schema() -> armature_workflow::schema::Schema {
    armature_workflow::schema::Schema::Record {
        fields: vec![
            armature_workflow::schema::Field {
                name: "id".to_string(),
                schema: armature_workflow::schema::Schema::String,
            },
            armature_workflow::schema::Field {
                name: "name".to_string(),
                schema: armature_workflow::schema::Schema::String,
            },
            armature_workflow::schema::Field {
                name: "status".to_string(),
                schema: armature_workflow::schema::Schema::String,
            },
            armature_workflow::schema::Field {
                name: "stdoutTail".to_string(),
                schema: armature_workflow::schema::Schema::String,
            },
            armature_workflow::schema::Field {
                name: "stderrTail".to_string(),
                schema: armature_workflow::schema::Schema::String,
            },
            armature_workflow::schema::Field {
                name: "exitCode".to_string(),
                schema: armature_workflow::schema::Schema::Optional {
                    inner: Box::new(armature_workflow::schema::Schema::Int),
                },
            },
        ],
    }
}

pub fn json_agent_finished_event_manifest() -> AdapterManifest {
    let mut events = BTreeMap::new();
    events.insert("finished".to_string(), json_agent_finished_event_schema());

    AdapterManifest {
        name: "json-agent-finished-events".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        types: BTreeMap::new(),
        effects: BTreeMap::new(),
        events,
    }
}

pub fn record_human_review_response(
    path: &Path,
    payload: &serde_json::Value,
) -> Result<(), armature_engine::effects::EffectError> {
    let review_id = payload
        .get("reviewId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            armature_engine::effects::EffectError::Unsupported(
                "humanReview.responded requires string field `reviewId`".to_string(),
            )
        })?;
    let decision = payload
        .get("decision")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            armature_engine::effects::EffectError::Unsupported(
                "humanReview.responded requires string field `decision`".to_string(),
            )
        })?;
    let response = payload
        .get("response")
        .filter(|value| !value.is_null())
        .cloned();

    let _lock = JsonFileLock::acquire(path)?;
    let mut document = read_json_document_or_default(path, serde_json::json!({ "reviews": [] }))?;
    let document_object = document.as_object_mut().ok_or_else(|| {
        armature_engine::effects::EffectError::Unsupported(
            "human review file root must be an object".to_string(),
        )
    })?;

    let reviews = document_object
        .entry("reviews")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let reviews = reviews.as_array_mut().ok_or_else(|| {
        armature_engine::effects::EffectError::Unsupported(
            "human review file field `reviews` must be an array".to_string(),
        )
    })?;

    for review in reviews.iter_mut() {
        if review.get("id").and_then(serde_json::Value::as_str) == Some(review_id) {
            let Some(review_object) = review.as_object_mut() else {
                continue;
            };
            review_object.insert(
                "status".to_string(),
                serde_json::Value::String("responded".to_string()),
            );
            review_object.insert(
                "decision".to_string(),
                serde_json::Value::String(decision.to_string()),
            );
            if let Some(response) = response.clone() {
                review_object.insert("response".to_string(), response);
            }
        }
    }

    let responses = document_object
        .entry("responses")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let responses = responses.as_array_mut().ok_or_else(|| {
        armature_engine::effects::EffectError::Unsupported(
            "human review file field `responses` must be an array".to_string(),
        )
    })?;
    if !responses.iter().any(|existing| {
        existing.get("reviewId").and_then(serde_json::Value::as_str) == Some(review_id)
    }) {
        responses.push(payload.clone());
    }

    write_json_document(path, &document)
}

pub fn record_agent_finished_event(
    path: &Path,
    payload: &serde_json::Value,
) -> Result<(), armature_engine::effects::EffectError> {
    let run_id = payload
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            armature_engine::effects::EffectError::Unsupported(
                "finished requires string field `id`".to_string(),
            )
        })?;
    let name = payload
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            armature_engine::effects::EffectError::Unsupported(
                "finished requires string field `name`".to_string(),
            )
        })?;
    let status = payload
        .get("status")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            armature_engine::effects::EffectError::Unsupported(
                "finished requires string field `status`".to_string(),
            )
        })?;

    let _lock = JsonFileLock::acquire(path)?;
    let mut document = read_json_document_or_default(
        path,
        serde_json::json!({
            "invocations": [],
            "messages": []
        }),
    )?;
    let document_object = document.as_object_mut().ok_or_else(|| {
        armature_engine::effects::EffectError::Unsupported(
            "agent file root must be an object".to_string(),
        )
    })?;

    let invocations = document_object
        .entry("invocations")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let invocations = invocations.as_array_mut().ok_or_else(|| {
        armature_engine::effects::EffectError::Unsupported(
            "agent file field `invocations` must be an array".to_string(),
        )
    })?;

    if let Some(invocation) = invocations.iter_mut().rev().find(|invocation| {
        invocation.get("id").and_then(serde_json::Value::as_str) == Some(run_id)
            || invocation
                .get("agent")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|agent| name == agent || name.starts_with(&format!("{agent}-")))
    }) {
        if let Some(invocation_object) = invocation.as_object_mut() {
            invocation_object.insert(
                "status".to_string(),
                serde_json::Value::String("finished".to_string()),
            );
            invocation_object.insert(
                "completion_id".to_string(),
                serde_json::Value::String(run_id.to_string()),
            );
            invocation_object.insert(
                "completion_status".to_string(),
                serde_json::Value::String(status.to_string()),
            );
            if let Some(exit_code) = payload.get("exitCode").filter(|value| !value.is_null()) {
                invocation_object.insert("exit_code".to_string(), exit_code.clone());
            }
        }
    }

    let completions = document_object
        .entry("completions")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let completions = completions.as_array_mut().ok_or_else(|| {
        armature_engine::effects::EffectError::Unsupported(
            "agent file field `completions` must be an array".to_string(),
        )
    })?;
    if !completions
        .iter()
        .any(|existing| existing.get("id").and_then(serde_json::Value::as_str) == Some(run_id))
    {
        completions.push(payload.clone());
    }

    write_json_document(path, &document)
}

pub fn validate_adapter_manifests(
    manifests: &[AdapterManifest],
) -> Vec<armature_workflow::Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut effect_names = BTreeMap::<&str, &str>::new();
    let mut event_names = BTreeMap::<&str, &str>::new();

    for manifest in manifests {
        if manifest.name.trim().is_empty() {
            diagnostics.push(error("adapter manifest name must not be empty".to_string()));
        }
        if manifest.version.trim().is_empty() {
            diagnostics.push(error(format!(
                "adapter manifest `{}` version must not be empty",
                manifest.name
            )));
        }

        for (type_name, schema) in &manifest.types {
            validate_identifier(
                &mut diagnostics,
                format!("adapter manifest `{}` type", manifest.name),
                type_name,
            );
            validate_schema_refs(
                &mut diagnostics,
                &manifest.name,
                &format!("type `{type_name}`"),
                schema,
                &manifest.types,
            );
            validate_schema_uniqueness(
                &mut diagnostics,
                &manifest.name,
                &format!("type `{type_name}`"),
                schema,
            );
            validate_map_key_schemas(
                &mut diagnostics,
                &manifest.name,
                &format!("type `{type_name}`"),
                schema,
                &manifest.types,
            );
        }
        validate_type_cycles(&mut diagnostics, &manifest.name, &manifest.types);

        for (effect_name, effect) in &manifest.effects {
            validate_effect_name(&mut diagnostics, &manifest.name, effect_name);
            if let Some(previous_manifest) =
                effect_names.insert(effect_name.as_str(), manifest.name.as_str())
            {
                diagnostics.push(error(format!(
                    "adapter effect `{effect_name}` is declared by both `{previous_manifest}` and `{}`",
                    manifest.name
                )));
            }
            validate_capability_list(
                &mut diagnostics,
                &manifest.name,
                effect_name,
                &effect.required_capabilities,
            );
            validate_token_list(
                &mut diagnostics,
                &manifest.name,
                effect_name,
                "failure category",
                &effect.failure_categories,
            );
            validate_schema_refs(
                &mut diagnostics,
                &manifest.name,
                &format!("effect `{effect_name}` input"),
                &effect.input,
                &manifest.types,
            );
            validate_schema_uniqueness(
                &mut diagnostics,
                &manifest.name,
                &format!("effect `{effect_name}` input"),
                &effect.input,
            );
            validate_map_key_schemas(
                &mut diagnostics,
                &manifest.name,
                &format!("effect `{effect_name}` input"),
                &effect.input,
                &manifest.types,
            );
            validate_schema_refs(
                &mut diagnostics,
                &manifest.name,
                &format!("effect `{effect_name}` output"),
                &effect.output,
                &manifest.types,
            );
            validate_schema_uniqueness(
                &mut diagnostics,
                &manifest.name,
                &format!("effect `{effect_name}` output"),
                &effect.output,
            );
            validate_map_key_schemas(
                &mut diagnostics,
                &manifest.name,
                &format!("effect `{effect_name}` output"),
                &effect.output,
                &manifest.types,
            );
            if !effect.idempotent
                && matches!(
                    effect.category,
                    armature_engine::effects::EffectCategory::AsyncInvocation
                        | armature_engine::effects::EffectCategory::Message
                        | armature_engine::effects::EffectCategory::HumanObligation
                )
            {
                diagnostics.push(error(format!(
                    "adapter manifest `{}` effect `{effect_name}` must be idempotent for post-commit dispatch",
                    manifest.name
                )));
            }
            if let Some(AdapterModel::NondeterministicOutcome { values }) = &effect.model {
                if values.is_empty() {
                    diagnostics.push(error(format!(
                        "adapter manifest `{}` effect `{effect_name}` nondeterministic model must list at least one value",
                        manifest.name
                    )));
                }
                validate_token_list(
                    &mut diagnostics,
                    &manifest.name,
                    effect_name,
                    "nondeterministic model value",
                    values,
                );
            }
        }

        for (event_name, schema) in &manifest.events {
            validate_event_name(&mut diagnostics, &manifest.name, event_name);
            if let Some(previous_manifest) =
                event_names.insert(event_name.as_str(), manifest.name.as_str())
            {
                diagnostics.push(error(format!(
                    "adapter event `{event_name}` is declared by both `{previous_manifest}` and `{}`",
                    manifest.name
                )));
            }
            validate_schema_refs(
                &mut diagnostics,
                &manifest.name,
                &format!("event `{event_name}` payload"),
                schema,
                &manifest.types,
            );
            validate_schema_uniqueness(
                &mut diagnostics,
                &manifest.name,
                &format!("event `{event_name}` payload"),
                schema,
            );
            validate_map_key_schemas(
                &mut diagnostics,
                &manifest.name,
                &format!("event `{event_name}` payload"),
                schema,
                &manifest.types,
            );
        }
    }

    diagnostics
}

pub fn validate_policy_documents(
    policies: &[CapabilityPolicyDocument],
) -> Vec<armature_workflow::Diagnostic> {
    let mut diagnostics = Vec::new();

    for (index, policy) in policies.iter().enumerate() {
        validate_policy_capability_list(
            &mut diagnostics,
            index,
            "allowed_capabilities",
            &policy.allowed_capabilities,
        );
        validate_policy_capability_list(
            &mut diagnostics,
            index,
            "denied_capabilities",
            &policy.denied_capabilities,
        );
        validate_policy_token_list(
            &mut diagnostics,
            index,
            "allowed_baml_urls",
            &policy.allowed_baml_urls,
        );
        validate_policy_token_list(
            &mut diagnostics,
            index,
            "allowed_models",
            &policy.allowed_models,
        );
        validate_policy_token_list(
            &mut diagnostics,
            index,
            "allowed_env_vars",
            &policy.allowed_env_vars,
        );

        let allowed = policy
            .allowed_capabilities
            .iter()
            .collect::<BTreeSet<&String>>();
        for capability in &policy.denied_capabilities {
            if allowed.contains(capability) {
                diagnostics.push(error(format!(
                    "policy document {index} lists capability `{capability}` in both allowed_capabilities and denied_capabilities"
                )));
            }
        }
    }

    diagnostics
}

pub fn validate_baml_http_policy(
    policies: &[CapabilityPolicyDocument],
    url: &str,
) -> Vec<armature_workflow::Diagnostic> {
    let mut diagnostics = Vec::new();
    let strictest_mode = policies
        .iter()
        .map(|policy| policy.mode)
        .max_by_key(policy_mode_rank)
        .unwrap_or(armature_workflow::policy::PolicyMode::Local);

    if policies.iter().any(|policy| {
        policy
            .denied_capabilities
            .iter()
            .any(|denied| denied == "baml.coerce")
    }) {
        diagnostics.push(error(
            "BAML HTTP coerce requires denied capability `baml.coerce`. Fix: remove `baml.coerce` from denied_capabilities only if model access is intended."
                .to_string(),
        ));
    } else if !policies.iter().any(|policy| {
        policy
            .allowed_capabilities
            .iter()
            .any(|allowed| allowed == "baml.coerce")
    }) {
        let severity = if unknown_capability_is_error(
            strictest_mode,
            armature_engine::effects::EffectCategory::SyncValue,
            "baml.coerce",
        ) {
            armature_workflow::Severity::Error
        } else {
            armature_workflow::Severity::Warning
        };
        diagnostics.push(diagnostic_at(
            severity,
            "BAML HTTP coerce requires capability `baml.coerce` that is not allowed by supplied policy. Fix: add `baml.coerce` to allowed_capabilities only if model access is intended."
                .to_string(),
            None,
        ));
    }

    if policies
        .iter()
        .any(|policy| policy.allow_baml_network == Some(false))
    {
        diagnostics.push(error(
            "BAML HTTP network execution is denied by supplied policy. Fix: set `allow_baml_network: true` only if network model access is intended."
                .to_string(),
        ));
    } else if !policies
        .iter()
        .any(|policy| policy.allow_baml_network == Some(true))
        && strictest_mode == armature_workflow::policy::PolicyMode::Enterprise
    {
        diagnostics.push(error(
            "BAML HTTP network execution requires `allow_baml_network: true` in enterprise policy. Fix: set `allow_baml_network: true` only for approved BAML HTTP endpoints."
                .to_string(),
        ));
    }

    let allowed_urls = policies
        .iter()
        .flat_map(|policy| policy.allowed_baml_urls.iter())
        .collect::<BTreeSet<&String>>();
    if !allowed_urls.is_empty() {
        if !allowed_urls.iter().any(|allowed| allowed.as_str() == url) {
            diagnostics.push(error(format!(
                "BAML HTTP URL `{url}` is not allowed by supplied policy. Fix: add the exact URL to allowed_baml_urls only if this endpoint is approved."
            )));
        }
    } else if strictest_mode == armature_workflow::policy::PolicyMode::Enterprise {
        diagnostics.push(error(format!(
            "BAML HTTP URL `{url}` requires an exact `allowed_baml_urls` entry in enterprise policy. Fix: add the exact URL to allowed_baml_urls only if this endpoint is approved."
        )));
    }

    diagnostics
}

pub fn should_store_baml_raw_response(policies: &[CapabilityPolicyDocument]) -> bool {
    if policies
        .iter()
        .any(|policy| policy.store_baml_raw_responses == Some(false))
    {
        return false;
    }
    if policies
        .iter()
        .any(|policy| policy.store_baml_raw_responses == Some(true))
    {
        return true;
    }
    let strictest_mode = policies
        .iter()
        .map(|policy| policy.mode)
        .max_by_key(policy_mode_rank)
        .unwrap_or(armature_workflow::policy::PolicyMode::Local);
    strictest_mode != armature_workflow::policy::PolicyMode::Enterprise
}

fn validate_type_cycles(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    manifest_name: &str,
    types: &BTreeMap<String, armature_workflow::schema::Schema>,
) {
    let mut visited = BTreeSet::new();
    let mut cyclic = BTreeSet::new();

    for type_name in types.keys() {
        let mut visiting = Vec::new();
        visit_type_refs(type_name, types, &mut visiting, &mut visited, &mut cyclic);
    }

    for type_name in cyclic {
        diagnostics.push(error(format!(
            "adapter manifest `{manifest_name}` type `{type_name}` has a cyclic reference"
        )));
    }
}

fn visit_type_refs(
    type_name: &str,
    types: &BTreeMap<String, armature_workflow::schema::Schema>,
    visiting: &mut Vec<String>,
    visited: &mut BTreeSet<String>,
    cyclic: &mut BTreeSet<String>,
) {
    if let Some(cycle_start) = visiting.iter().position(|name| name == type_name) {
        cyclic.extend(visiting[cycle_start..].iter().cloned());
        return;
    }
    if visited.contains(type_name) {
        return;
    }

    visiting.push(type_name.to_string());
    if let Some(schema) = types.get(type_name) {
        let mut refs = Vec::new();
        collect_schema_refs(schema, &mut refs);
        for ref_name in refs {
            if types.contains_key(ref_name) {
                visit_type_refs(ref_name, types, visiting, visited, cyclic);
            }
        }
    }
    visiting.pop();
    visited.insert(type_name.to_string());
}

fn collect_schema_refs<'a>(schema: &'a armature_workflow::schema::Schema, refs: &mut Vec<&'a str>) {
    match schema {
        armature_workflow::schema::Schema::Ref { name } => refs.push(name),
        armature_workflow::schema::Schema::Optional { inner }
        | armature_workflow::schema::Schema::List { inner }
        | armature_workflow::schema::Schema::Set { inner } => {
            collect_schema_refs(inner, refs);
        }
        armature_workflow::schema::Schema::Map { key, value } => {
            collect_schema_refs(key, refs);
            collect_schema_refs(value, refs);
        }
        armature_workflow::schema::Schema::Union { variants } => {
            for variant in variants {
                collect_schema_refs(variant, refs);
            }
        }
        armature_workflow::schema::Schema::Record { fields } => {
            for field in fields {
                collect_schema_refs(&field.schema, refs);
            }
        }
        armature_workflow::schema::Schema::String
        | armature_workflow::schema::Schema::Int
        | armature_workflow::schema::Schema::Float
        | armature_workflow::schema::Schema::Boolean
        | armature_workflow::schema::Schema::Null
        | armature_workflow::schema::Schema::Time
        | armature_workflow::schema::Schema::Duration
        | armature_workflow::schema::Schema::Agent
        | armature_workflow::schema::Schema::Literal { .. }
        | armature_workflow::schema::Schema::Enum { .. }
        | armature_workflow::schema::Schema::Json => {}
    }
}

pub fn validate_workflow_effects(
    ir: &armature_workflow::WorkflowIr,
    manifests: &[AdapterManifest],
) -> Vec<armature_workflow::Diagnostic> {
    let mut diagnostics = Vec::new();
    let registry = EffectRegistry::new(manifests, &mut diagnostics);

    validate_invariant_effects(&mut diagnostics, &registry, ir);

    for (state_name, state) in &ir.statechart.states {
        validate_state_effects(&mut diagnostics, &registry, ir, state_name, state);
    }
    validate_adapter_event_compatibility(&mut diagnostics, ir, manifests);

    diagnostics
}

pub fn validate_workflow_policy(
    ir: &armature_workflow::WorkflowIr,
    manifests: &[AdapterManifest],
    policies: &[CapabilityPolicyDocument],
) -> Vec<armature_workflow::Diagnostic> {
    let mut diagnostics = validate_policy_documents(policies);
    if policies.is_empty() {
        return diagnostics;
    }

    let registry = EffectRegistry::new(manifests, &mut diagnostics);
    validate_invariant_policy(&mut diagnostics, &registry, policies, ir);
    for (state_name, state) in &ir.statechart.states {
        validate_state_policy(&mut diagnostics, &registry, policies, ir, state_name, state);
    }

    diagnostics
}

fn validate_adapter_event_compatibility(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    ir: &armature_workflow::WorkflowIr,
    manifests: &[AdapterManifest],
) {
    for manifest in manifests {
        for (event_name, adapter_schema) in &manifest.events {
            let Some(workflow_event) = ir.events.get(event_name) else {
                continue;
            };
            if !schemas_equivalent(
                &workflow_event.payload,
                &ir.types,
                adapter_schema,
                &manifest.types,
                0,
            ) {
                diagnostics.push(error(format!(
                    "adapter manifest `{}` event `{event_name}` schema does not match workflow event schema",
                    manifest.name
                )));
            }
        }
    }
}

#[derive(Clone, Copy)]
struct RegisteredEffect<'a> {
    manifest_name: &'a str,
    types: &'a BTreeMap<String, armature_workflow::schema::Schema>,
    effect: &'a AdapterEffect,
}

struct EffectRegistry<'a> {
    effects: BTreeMap<&'a str, RegisteredEffect<'a>>,
}

impl<'a> EffectRegistry<'a> {
    fn new(
        manifests: &'a [AdapterManifest],
        diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    ) -> Self {
        let mut effects = BTreeMap::new();
        let mut duplicates = BTreeSet::new();

        for manifest in manifests {
            for (name, effect) in &manifest.effects {
                let registered = RegisteredEffect {
                    manifest_name: &manifest.name,
                    types: &manifest.types,
                    effect,
                };
                if effects.insert(name.as_str(), registered).is_some() {
                    duplicates.insert(name.clone());
                }
            }
        }

        for duplicate in duplicates {
            diagnostics.push(error(format!(
                "adapter effect `{duplicate}` is declared by more than one manifest"
            )));
        }

        Self { effects }
    }

    fn effect(&self, name: &str) -> Option<RegisteredEffect<'a>> {
        self.effects.get(name).copied()
    }
}

fn validate_state_effects(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    state_name: &str,
    state: &armature_workflow::ir::State,
) {
    let owner = format!("state `{state_name}`");

    for handler in &state.on {
        if let Some(guard) = &handler.guard {
            validate_expr_effects(
                diagnostics,
                registry,
                ir,
                &owner,
                guard,
                handler.span.as_ref(),
            );
            let guard_scope = AdapterExprScope {
                event_name: Some(handler.event.clone()),
                event_binding: handler.binding.clone(),
                locals: BTreeMap::new(),
            };
            validate_bool_adapter_expr(
                diagnostics,
                registry,
                ir,
                guard,
                BoolExprExpectation {
                    owner: &owner,
                    label: "handler guard",
                    span: handler.span.as_ref(),
                    scope: &guard_scope,
                },
            );
        }
        let mut scope = AdapterExprScope {
            event_name: Some(handler.event.clone()),
            event_binding: handler.binding.clone(),
            locals: BTreeMap::new(),
        };
        validate_steps_adapter_types(
            diagnostics,
            registry,
            ir,
            state_name,
            &handler.steps,
            &mut scope,
        );
        validate_steps_effects(diagnostics, registry, ir, &owner, &handler.steps);
    }
    for transition in &state.always {
        if let Some(guard) = &transition.guard {
            validate_expr_effects(
                diagnostics,
                registry,
                ir,
                &owner,
                guard,
                transition.span.as_ref(),
            );
            validate_bool_adapter_expr(
                diagnostics,
                registry,
                ir,
                guard,
                BoolExprExpectation {
                    owner: &owner,
                    label: "always guard",
                    span: transition.span.as_ref(),
                    scope: &AdapterExprScope::default(),
                },
            );
        }
        let mut scope = AdapterExprScope::default();
        validate_steps_adapter_types(
            diagnostics,
            registry,
            ir,
            state_name,
            &transition.steps,
            &mut scope,
        );
        validate_steps_effects(diagnostics, registry, ir, &owner, &transition.steps);
    }
    let mut entry_scope = AdapterExprScope::default();
    validate_steps_adapter_types(
        diagnostics,
        registry,
        ir,
        state_name,
        &state.entry,
        &mut entry_scope,
    );
    validate_steps_effects(diagnostics, registry, ir, &owner, &state.entry);

    for (child_name, child) in &state.states {
        validate_state_effects(diagnostics, registry, ir, child_name, child);
    }
}

fn validate_invariant_effects(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
) {
    for invariant in &ir.invariants {
        let armature_workflow::ir::Invariant::Expression { name, expr, span } = invariant else {
            continue;
        };
        let owner = format!("invariant `{name}`");
        let scope = AdapterExprScope::default();
        validate_expr_effects(diagnostics, registry, ir, &owner, expr, span.as_ref());
        validate_bool_adapter_expr(
            diagnostics,
            registry,
            ir,
            expr,
            BoolExprExpectation {
                owner: &owner,
                label: "expression",
                span: span.as_ref(),
                scope: &scope,
            },
        );
    }
}

fn validate_steps_effects(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    owner: &str,
    steps: &[armature_workflow::ir::Step],
) {
    for step in steps {
        if let Some(request) = adapter_effect_request_shape(ir, step) {
            validate_effect_shape(
                diagnostics,
                registry,
                owner,
                request.as_ref(),
                step.span.as_ref(),
            );
        }

        for value in step.args.values() {
            if let Ok(expr) = serde_json::from_value::<armature_workflow::expr::Expr>(value.clone())
            {
                validate_expr_effects(diagnostics, registry, ir, owner, &expr, step.span.as_ref());
            } else if let Ok(exprs) =
                serde_json::from_value::<Vec<armature_workflow::expr::Expr>>(value.clone())
            {
                for expr in &exprs {
                    validate_expr_effects(
                        diagnostics,
                        registry,
                        ir,
                        owner,
                        expr,
                        step.span.as_ref(),
                    );
                }
            }
        }

        for arm in &step.case_arms {
            validate_steps_effects(diagnostics, registry, ir, owner, &arm.steps);
        }
    }
}

#[derive(Clone, Default)]
struct AdapterExprScope {
    event_name: Option<String>,
    event_binding: Option<String>,
    locals: BTreeMap<String, TypedSchema>,
}

#[derive(Clone)]
struct TypedSchema {
    schema: armature_workflow::schema::Schema,
    types: BTreeMap<String, armature_workflow::schema::Schema>,
}

fn validate_steps_adapter_types(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    state_name: &str,
    steps: &[armature_workflow::ir::Step],
    scope: &mut AdapterExprScope,
) {
    let owner = format!("state `{state_name}`");

    for step in steps {
        validate_step_request_adapter_input(diagnostics, registry, ir, state_name, step, scope);

        match step.effect.as_str() {
            "send" => validate_step_expr_schema(
                diagnostics,
                registry,
                ir,
                state_name,
                step,
                scope,
                StepExprExpectation {
                    arg: "message",
                    expected: workflow_schema(armature_workflow::schema::Schema::String, ir),
                },
            ),
            "askHuman" => validate_step_expr_schema(
                diagnostics,
                registry,
                ir,
                state_name,
                step,
                scope,
                StepExprExpectation {
                    arg: "reason",
                    expected: workflow_schema(armature_workflow::schema::Schema::String, ir),
                },
            ),
            "assign" => {
                if let Some(target) = step.args.get("target").and_then(|value| value.as_str()) {
                    if let Some(expected) = resolve_typed_path(ir, scope, target) {
                        validate_step_expr_schema(
                            diagnostics,
                            registry,
                            ir,
                            state_name,
                            step,
                            scope,
                            StepExprExpectation {
                                arg: "value",
                                expected,
                            },
                        );
                    }
                }
            }
            "raise" => {
                if let Some(event) = step.args.get("event").and_then(|value| value.as_str()) {
                    if let Some(expected) = ir
                        .events
                        .get(event)
                        .map(|event| workflow_schema(event.payload.clone(), ir))
                    {
                        validate_step_expr_schema(
                            diagnostics,
                            registry,
                            ir,
                            state_name,
                            step,
                            scope,
                            StepExprExpectation {
                                arg: "payload",
                                expected,
                            },
                        );
                    }
                }
            }
            "let" => {
                if let Some(expr) = step_expr(step, "value") {
                    validate_adapter_expr_constraints(
                        diagnostics,
                        registry,
                        ir,
                        &owner,
                        &expr,
                        step.span.as_ref(),
                        scope,
                    );
                    if let Some(local) = &step.assign {
                        if let Some(schema) = infer_adapter_expr_schema(registry, ir, scope, &expr)
                        {
                            scope.locals.insert(local.clone(), schema);
                        }
                    }
                }
            }
            "case" => {
                if let Some(expr) = step_expr(step, "expr") {
                    validate_adapter_expr_constraints(
                        diagnostics,
                        registry,
                        ir,
                        &owner,
                        &expr,
                        step.span.as_ref(),
                        scope,
                    );
                }
                for arm in &step.case_arms {
                    let mut arm_scope = scope.clone();
                    validate_steps_adapter_types(
                        diagnostics,
                        registry,
                        ir,
                        state_name,
                        &arm.steps,
                        &mut arm_scope,
                    );
                }
            }
            _ => {}
        }
    }
}

fn validate_step_request_adapter_input(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    state_name: &str,
    step: &armature_workflow::ir::Step,
    scope: &AdapterExprScope,
) {
    if is_native_agent_step(ir, step) {
        return;
    }
    let Some(request) = precise_adapter_effect_request_shape(registry, ir, scope, step) else {
        return;
    };
    let Some(registered) = registry.effect(&request.effect_name) else {
        return;
    };
    if registered.effect.category != request.category {
        return;
    }

    if !adapter_schema_accepts_schema(
        &registered.effect.input,
        registered.types,
        &request.input.schema,
        &request.input.types,
        0,
    ) {
        diagnostics.push(error_at(
            format!(
                "state `{state_name}` effect `{}` request args have schema `{}`, but adapter manifest `{}` expects `{}` after adapter output inference",
                request.effect_name,
                adapter_schema_kind(&request.input.schema),
                registered.manifest_name,
                adapter_schema_kind(&registered.effect.input)
            ),
            step.span.as_ref(),
        ));
    }
}

struct PreciseAdapterEffectRequestShape {
    effect_name: String,
    category: armature_engine::effects::EffectCategory,
    input: TypedSchema,
}

fn precise_adapter_effect_request_shape(
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    scope: &AdapterExprScope,
    step: &armature_workflow::ir::Step,
) -> Option<PreciseAdapterEffectRequestShape> {
    use armature_workflow::schema::{Field, Schema};

    fn field(name: &str, schema: Schema) -> Field {
        Field {
            name: name.to_string(),
            schema,
        }
    }

    fn expr_field(
        registry: &EffectRegistry<'_>,
        ir: &armature_workflow::WorkflowIr,
        scope: &AdapterExprScope,
        step: &armature_workflow::ir::Step,
        name: &str,
    ) -> Option<Field> {
        let expr = step_expr(step, name)?;
        let typed = infer_adapter_expr_schema(registry, ir, scope, &expr)
            .unwrap_or_else(|| workflow_schema(Schema::Json, ir));
        Some(field(
            name,
            materialize_schema_refs(&typed.schema, &typed.types, 0),
        ))
    }

    let (effect_name, category, fields) = match step.effect.as_str() {
        "send" => {
            let mut fields = vec![field("agent", Schema::String)];
            if let Some(message) = expr_field(registry, ir, scope, step, "message") {
                fields.push(message);
            }
            (
                "send".to_string(),
                armature_engine::effects::EffectCategory::Message,
                fields,
            )
        }
        "start" => {
            let mut fields = vec![field("agent", Schema::String)];
            if let Some(input) = expr_field(registry, ir, scope, step, "input") {
                fields.push(input);
            }
            (
                "start".to_string(),
                armature_engine::effects::EffectCategory::AsyncInvocation,
                fields,
            )
        }
        "askHuman" => {
            let mut fields = Vec::new();
            if let Some(reason) = expr_field(registry, ir, scope, step, "reason") {
                fields.push(reason);
            }
            (
                "askHuman".to_string(),
                armature_engine::effects::EffectCategory::HumanObligation,
                fields,
            )
        }
        "capability_call" => {
            let capability = step
                .args
                .get("capability")
                .and_then(|value| value.as_str())?;
            let operation = step
                .args
                .get("operation")
                .and_then(|value| value.as_str())
                .unwrap_or("call");
            let call_arg_schema = step
                .args
                .get("call_args")
                .and_then(|value| value.as_array())
                .map(|values| {
                    infer_list_item_schema(values.iter().filter_map(|value| {
                        serde_json::from_value::<armature_workflow::expr::Expr>(value.clone())
                            .ok()
                            .and_then(|expr| infer_adapter_expr_schema(registry, ir, scope, &expr))
                            .map(|typed| materialize_schema_refs(&typed.schema, &typed.types, 0))
                    }))
                })
                .unwrap_or(Schema::Json);
            (
                format!("{capability}.{operation}"),
                armature_engine::effects::EffectCategory::SyncValue,
                vec![
                    field("capability", Schema::String),
                    field("operation", Schema::String),
                    field(
                        "call_args",
                        Schema::List {
                            inner: Box::new(call_arg_schema),
                        },
                    ),
                ],
            )
        }
        _ => return None,
    };

    Some(PreciseAdapterEffectRequestShape {
        effect_name,
        category,
        input: TypedSchema {
            schema: Schema::Record { fields },
            types: BTreeMap::new(),
        },
    })
}

struct StepExprExpectation {
    arg: &'static str,
    expected: TypedSchema,
}

fn validate_step_expr_schema(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    state_name: &str,
    step: &armature_workflow::ir::Step,
    scope: &AdapterExprScope,
    expectation: StepExprExpectation,
) {
    let Some(expr) = step_expr(step, expectation.arg) else {
        return;
    };
    let owner = format!("state `{state_name}`");

    validate_adapter_expr_constraints(
        diagnostics,
        registry,
        ir,
        &owner,
        &expr,
        step.span.as_ref(),
        scope,
    );

    let Some(actual) = infer_adapter_expr_schema(registry, ir, scope, &expr) else {
        return;
    };

    if !adapter_schema_accepts_schema(
        &expectation.expected.schema,
        &expectation.expected.types,
        &actual.schema,
        &actual.types,
        0,
    ) {
        diagnostics.push(error_at(
            format!(
                "state `{state_name}` `{}` {} has `{}` value after adapter output inference; expected `{}`",
                step.effect,
                expectation.arg,
                adapter_schema_kind(&actual.schema),
                adapter_schema_kind(&expectation.expected.schema)
            ),
            step.span.as_ref(),
        ));
    }
}

struct BoolExprExpectation<'a> {
    owner: &'a str,
    label: &'a str,
    span: Option<&'a armature_workflow::SourceSpan>,
    scope: &'a AdapterExprScope,
}

fn validate_bool_adapter_expr(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    expr: &armature_workflow::expr::Expr,
    expectation: BoolExprExpectation<'_>,
) {
    validate_adapter_expr_constraints(
        diagnostics,
        registry,
        ir,
        expectation.owner,
        expr,
        expectation.span,
        expectation.scope,
    );

    let Some(actual) = infer_adapter_expr_schema(registry, ir, expectation.scope, expr) else {
        return;
    };
    let expected = workflow_schema(armature_workflow::schema::Schema::Boolean, ir);
    if !adapter_schema_accepts_schema(
        &expected.schema,
        &expected.types,
        &actual.schema,
        &actual.types,
        0,
    ) {
        diagnostics.push(error_at(
            format!(
                "{} {} has `{}` value after adapter output inference; expected `bool`",
                expectation.owner,
                expectation.label,
                adapter_schema_kind(&actual.schema)
            ),
            expectation.span,
        ));
    }
}

fn validate_adapter_expr_constraints(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    owner: &str,
    expr: &armature_workflow::expr::Expr,
    span: Option<&armature_workflow::SourceSpan>,
    scope: &AdapterExprScope,
) {
    use armature_workflow::expr::Expr;
    use armature_workflow::schema::Schema;

    match expr {
        Expr::Eq { left, right } | Expr::Neq { left, right } => {
            validate_adapter_expr_constraints(diagnostics, registry, ir, owner, left, span, scope);
            validate_adapter_expr_constraints(diagnostics, registry, ir, owner, right, span, scope);
            let Some(left_schema) = infer_adapter_expr_schema(registry, ir, scope, left) else {
                return;
            };
            let Some(right_schema) = infer_adapter_expr_schema(registry, ir, scope, right) else {
                return;
            };
            if !typed_schema_accepts(&left_schema, &right_schema)
                && !typed_schema_accepts(&right_schema, &left_schema)
            {
                diagnostics.push(error_at(
                    format!(
                        "{owner} compares `{}` and `{}` values after adapter output inference",
                        adapter_schema_kind(&left_schema.schema),
                        adapter_schema_kind(&right_schema.schema)
                    ),
                    span,
                ));
            }
        }
        Expr::In { left, right } => {
            validate_adapter_expr_constraints(diagnostics, registry, ir, owner, left, span, scope);
            validate_adapter_expr_constraints(diagnostics, registry, ir, owner, right, span, scope);
            let Some(right_schema) = infer_adapter_expr_schema(registry, ir, scope, right) else {
                return;
            };
            let item_schema = match &right_schema.schema {
                Schema::List { inner } | Schema::Set { inner } => TypedSchema {
                    schema: (*inner.clone()),
                    types: right_schema.types.clone(),
                },
                Schema::Json => return,
                _ => {
                    diagnostics.push(error_at(
                        format!(
                            "{owner} uses `in` with `{}` right-hand value after adapter output inference; expected list or set",
                            adapter_schema_kind(&right_schema.schema)
                        ),
                        span,
                    ));
                    return;
                }
            };
            let Some(left_schema) = infer_adapter_expr_schema(registry, ir, scope, left) else {
                return;
            };
            if !typed_schema_accepts(&item_schema, &left_schema) {
                diagnostics.push(error_at(
                    format!(
                        "{owner} uses `in` with `{}` item after adapter output inference; expected `{}`",
                        adapter_schema_kind(&left_schema.schema),
                        adapter_schema_kind(&item_schema.schema)
                    ),
                    span,
                ));
            }
        }
        Expr::Lt { left, right }
        | Expr::Lte { left, right }
        | Expr::Gt { left, right }
        | Expr::Gte { left, right } => {
            validate_adapter_expr_constraints(diagnostics, registry, ir, owner, left, span, scope);
            validate_adapter_expr_constraints(diagnostics, registry, ir, owner, right, span, scope);
            validate_ordered_adapter_operand(diagnostics, registry, ir, owner, left, span, scope);
            validate_ordered_adapter_operand(diagnostics, registry, ir, owner, right, span, scope);
        }
        Expr::And { exprs } | Expr::Or { exprs } => {
            for expr in exprs {
                validate_adapter_expr_constraints(
                    diagnostics,
                    registry,
                    ir,
                    owner,
                    expr,
                    span,
                    scope,
                );
                validate_boolean_adapter_operand(
                    diagnostics,
                    registry,
                    ir,
                    owner,
                    expr,
                    span,
                    scope,
                );
            }
        }
        Expr::Not { expr } => {
            validate_adapter_expr_constraints(diagnostics, registry, ir, owner, expr, span, scope);
            validate_boolean_adapter_operand(diagnostics, registry, ir, owner, expr, span, scope);
        }
        Expr::Object { fields } => {
            for expr in fields.values() {
                validate_adapter_expr_constraints(
                    diagnostics,
                    registry,
                    ir,
                    owner,
                    expr,
                    span,
                    scope,
                );
            }
        }
        Expr::List { items } => {
            for expr in items {
                validate_adapter_expr_constraints(
                    diagnostics,
                    registry,
                    ir,
                    owner,
                    expr,
                    span,
                    scope,
                );
            }
        }
        Expr::Call { args, .. } => {
            validate_capability_value_call_input(
                diagnostics,
                registry,
                ir,
                owner,
                expr,
                span,
                scope,
            );
            for arg in args {
                validate_adapter_expr_constraints(
                    diagnostics,
                    registry,
                    ir,
                    owner,
                    arg,
                    span,
                    scope,
                );
            }
        }
        Expr::Literal { .. } | Expr::Path { .. } => {}
    }
}

fn validate_capability_value_call_input(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    owner: &str,
    expr: &armature_workflow::expr::Expr,
    span: Option<&armature_workflow::SourceSpan>,
    scope: &AdapterExprScope,
) {
    let armature_workflow::expr::Expr::Call { name, args } = expr else {
        return;
    };
    if !name
        .split_once('.')
        .is_some_and(|(capability, _)| ir.capabilities.contains_key(capability))
    {
        return;
    }
    let Some(registered) = registry.effect(name) else {
        return;
    };
    if registered.effect.category != armature_engine::effects::EffectCategory::SyncValue {
        return;
    }

    let actual = capability_value_call_input_schema(registry, ir, scope, args);
    if !typed_schema_accepts(
        &TypedSchema {
            schema: registered.effect.input.clone(),
            types: registered.types.clone(),
        },
        &actual,
    ) {
        diagnostics.push(error_at(
            format!(
                "{owner} capability value call `{name}` input has `{}` schema after adapter output inference; adapter manifest `{}` expects `{}`",
                adapter_schema_kind(&actual.schema),
                registered.manifest_name,
                adapter_schema_kind(&registered.effect.input)
            ),
            span,
        ));
    }
}

fn capability_value_call_input_schema(
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    scope: &AdapterExprScope,
    args: &[armature_workflow::expr::Expr],
) -> TypedSchema {
    use armature_workflow::schema::Schema;

    match args {
        [] => workflow_schema(Schema::Record { fields: Vec::new() }, ir),
        [arg] => infer_adapter_expr_schema(registry, ir, scope, arg)
            .map(|typed| TypedSchema {
                schema: materialize_schema_refs(&typed.schema, &typed.types, 0),
                types: BTreeMap::new(),
            })
            .unwrap_or_else(|| workflow_schema(Schema::Json, ir)),
        args => workflow_schema(
            Schema::List {
                inner: Box::new(infer_list_item_schema(args.iter().filter_map(|arg| {
                    infer_adapter_expr_schema(registry, ir, scope, arg)
                        .map(|typed| materialize_schema_refs(&typed.schema, &typed.types, 0))
                }))),
            },
            ir,
        ),
    }
}

fn validate_boolean_adapter_operand(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    owner: &str,
    expr: &armature_workflow::expr::Expr,
    span: Option<&armature_workflow::SourceSpan>,
    scope: &AdapterExprScope,
) {
    let Some(actual) = infer_adapter_expr_schema(registry, ir, scope, expr) else {
        return;
    };
    let expected = workflow_schema(armature_workflow::schema::Schema::Boolean, ir);
    if !typed_schema_accepts(&expected, &actual) {
        diagnostics.push(error_at(
            format!(
                "{owner} boolean expression has `{}` operand after adapter output inference; expected `bool`",
                adapter_schema_kind(&actual.schema)
            ),
            span,
        ));
    }
}

fn validate_ordered_adapter_operand(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    owner: &str,
    expr: &armature_workflow::expr::Expr,
    span: Option<&armature_workflow::SourceSpan>,
    scope: &AdapterExprScope,
) {
    let Some(actual) = infer_adapter_expr_schema(registry, ir, scope, expr) else {
        return;
    };
    if adapter_schema_is_orderable(&actual.schema) {
        return;
    }
    diagnostics.push(error_at(
        format!(
            "{owner} ordered comparison has `{}` operand after adapter output inference; expected number, string, time, or duration",
            adapter_schema_kind(&actual.schema)
        ),
        span,
    ));
}

fn adapter_schema_is_orderable(schema: &armature_workflow::schema::Schema) -> bool {
    match schema {
        armature_workflow::schema::Schema::String
        | armature_workflow::schema::Schema::Time
        | armature_workflow::schema::Schema::Duration
        | armature_workflow::schema::Schema::Int
        | armature_workflow::schema::Schema::Float
        | armature_workflow::schema::Schema::Json => true,
        armature_workflow::schema::Schema::Literal { value } => {
            value.is_string() || value.is_number()
        }
        armature_workflow::schema::Schema::Union { variants } => {
            variants.iter().all(adapter_schema_is_orderable)
        }
        armature_workflow::schema::Schema::Optional { inner } => adapter_schema_is_orderable(inner),
        _ => false,
    }
}

fn typed_schema_accepts(expected: &TypedSchema, actual: &TypedSchema) -> bool {
    adapter_schema_accepts_schema(
        &expected.schema,
        &expected.types,
        &actual.schema,
        &actual.types,
        0,
    )
}

fn infer_adapter_expr_schema(
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    scope: &AdapterExprScope,
    expr: &armature_workflow::expr::Expr,
) -> Option<TypedSchema> {
    use armature_workflow::expr::Expr;
    use armature_workflow::schema::{Field, Schema};

    match expr {
        Expr::Literal { value } => Some(workflow_schema(infer_literal_schema(value), ir)),
        Expr::Path { path } => resolve_typed_path(ir, scope, path),
        Expr::Object { fields } => Some(workflow_schema(
            Schema::Record {
                fields: fields
                    .iter()
                    .map(|(name, expr)| Field {
                        name: name.clone(),
                        schema: infer_adapter_expr_schema(registry, ir, scope, expr)
                            .map(|schema| schema.schema)
                            .unwrap_or(Schema::Json),
                    })
                    .collect(),
            },
            ir,
        )),
        Expr::List { items } => Some(workflow_schema(
            Schema::List {
                inner: Box::new(infer_list_item_schema(items.iter().filter_map(|expr| {
                    infer_adapter_expr_schema(registry, ir, scope, expr).map(|schema| schema.schema)
                }))),
            },
            ir,
        )),
        Expr::Call { name, .. } => infer_adapter_call_schema(registry, ir, scope, name),
        Expr::Eq { .. }
        | Expr::Neq { .. }
        | Expr::Lt { .. }
        | Expr::Lte { .. }
        | Expr::Gt { .. }
        | Expr::Gte { .. }
        | Expr::In { .. }
        | Expr::And { .. }
        | Expr::Or { .. }
        | Expr::Not { .. } => Some(workflow_schema(Schema::Boolean, ir)),
    }
}

fn infer_adapter_call_schema(
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    scope: &AdapterExprScope,
    name: &str,
) -> Option<TypedSchema> {
    use armature_workflow::schema::Schema;

    if let Some(function_name) = name.strip_prefix("coerce ") {
        return ir
            .coerce_functions
            .get(function_name.trim())
            .map(|function| workflow_schema(function.output.clone(), ir));
    }
    if let Some(function) = ir.coerce_functions.get(name) {
        return Some(workflow_schema(function.output.clone(), ir));
    }
    if name == "now" {
        return Some(workflow_schema(Schema::Time, ir));
    }
    if name == "elapsedSince" {
        return Some(workflow_schema(Schema::Duration, ir));
    }
    if let Some(receiver) = name.strip_suffix(".append") {
        return resolve_typed_path(ir, scope, receiver);
    }
    if name
        .split_once('.')
        .is_some_and(|(capability, _)| ir.capabilities.contains_key(capability))
    {
        return registry.effect(name).map(|registered| TypedSchema {
            schema: registered.effect.output.clone(),
            types: registered.types.clone(),
        });
    }

    Some(workflow_schema(Schema::Json, ir))
}

fn step_expr(
    step: &armature_workflow::ir::Step,
    arg: &str,
) -> Option<armature_workflow::expr::Expr> {
    step.args
        .get(arg)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn workflow_schema(
    schema: armature_workflow::schema::Schema,
    ir: &armature_workflow::WorkflowIr,
) -> TypedSchema {
    TypedSchema {
        schema,
        types: ir.types.clone(),
    }
}

fn materialize_schema_refs(
    schema: &armature_workflow::schema::Schema,
    types: &BTreeMap<String, armature_workflow::schema::Schema>,
    depth: usize,
) -> armature_workflow::schema::Schema {
    use armature_workflow::schema::Schema;

    if depth > 16 {
        return Schema::Json;
    }

    match schema {
        Schema::Ref { name } => types
            .get(name)
            .map(|schema| materialize_schema_refs(schema, types, depth + 1))
            .unwrap_or_else(|| schema.clone()),
        Schema::Optional { inner } => Schema::Optional {
            inner: Box::new(materialize_schema_refs(inner, types, depth + 1)),
        },
        Schema::List { inner } => Schema::List {
            inner: Box::new(materialize_schema_refs(inner, types, depth + 1)),
        },
        Schema::Set { inner } => Schema::Set {
            inner: Box::new(materialize_schema_refs(inner, types, depth + 1)),
        },
        Schema::Map { key, value } => Schema::Map {
            key: Box::new(materialize_schema_refs(key, types, depth + 1)),
            value: Box::new(materialize_schema_refs(value, types, depth + 1)),
        },
        Schema::Union { variants } => Schema::Union {
            variants: variants
                .iter()
                .map(|variant| materialize_schema_refs(variant, types, depth + 1))
                .collect(),
        },
        Schema::Record { fields } => Schema::Record {
            fields: fields
                .iter()
                .map(|field| armature_workflow::schema::Field {
                    name: field.name.clone(),
                    schema: materialize_schema_refs(&field.schema, types, depth + 1),
                })
                .collect(),
        },
        Schema::String
        | Schema::Int
        | Schema::Float
        | Schema::Boolean
        | Schema::Null
        | Schema::Time
        | Schema::Duration
        | Schema::Agent
        | Schema::Literal { .. }
        | Schema::Enum { .. }
        | Schema::Json => schema.clone(),
    }
}

fn infer_literal_schema(value: &serde_json::Value) -> armature_workflow::schema::Schema {
    use armature_workflow::schema::Schema;

    match value {
        serde_json::Value::String(_)
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::Null => Schema::Literal {
            value: value.clone(),
        },
        serde_json::Value::Array(items) => Schema::List {
            inner: Box::new(infer_list_item_schema(
                items.iter().map(infer_literal_schema),
            )),
        },
        serde_json::Value::Object(entries) => Schema::Record {
            fields: entries
                .iter()
                .map(|(name, value)| armature_workflow::schema::Field {
                    name: name.clone(),
                    schema: infer_literal_schema(value),
                })
                .collect(),
        },
    }
}

fn infer_list_item_schema(
    schemas: impl IntoIterator<Item = armature_workflow::schema::Schema>,
) -> armature_workflow::schema::Schema {
    let mut schemas = schemas.into_iter();
    let Some(first) = schemas.next() else {
        return armature_workflow::schema::Schema::Json;
    };

    let mut variants = vec![first];
    for schema in schemas {
        if !variants.contains(&schema) {
            variants.push(schema);
        }
    }

    if variants.len() == 1 {
        variants.remove(0)
    } else {
        armature_workflow::schema::Schema::Union { variants }
    }
}

fn resolve_typed_path(
    ir: &armature_workflow::WorkflowIr,
    scope: &AdapterExprScope,
    path: &str,
) -> Option<TypedSchema> {
    let (root, rest) = path.split_once('.').unwrap_or((path, ""));

    if root == "data" {
        let (field, nested) = rest.split_once('.').unwrap_or((rest, ""));
        let schema = ir.context_schema.get(field)?;
        return resolve_typed_schema_path(&workflow_schema(schema.clone(), ir), nested);
    }

    if scope.event_binding.as_deref() == Some(root) {
        let event_name = scope.event_name.as_ref()?;
        let schema = &ir.events.get(event_name)?.payload;
        return resolve_typed_schema_path(&workflow_schema(schema.clone(), ir), rest);
    }

    if let Some(schema) = scope.locals.get(root) {
        return resolve_typed_schema_path(schema, rest);
    }

    None
}

fn resolve_typed_schema_path(schema: &TypedSchema, nested_path: &str) -> Option<TypedSchema> {
    if nested_path.is_empty() {
        return Some(schema.clone());
    }
    resolve_typed_schema_path_inner(&schema.schema, &schema.types, nested_path, 0).map(|resolved| {
        TypedSchema {
            schema: resolved,
            types: schema.types.clone(),
        }
    })
}

fn resolve_typed_schema_path_inner(
    schema: &armature_workflow::schema::Schema,
    types: &BTreeMap<String, armature_workflow::schema::Schema>,
    nested_path: &str,
    depth: usize,
) -> Option<armature_workflow::schema::Schema> {
    if nested_path.is_empty() {
        return Some(schema.clone());
    }
    if depth > 16 {
        return Some(armature_workflow::schema::Schema::Json);
    }

    match schema {
        armature_workflow::schema::Schema::Optional { inner } => {
            resolve_typed_schema_path_inner(inner, types, nested_path, depth + 1)
        }
        armature_workflow::schema::Schema::Ref { name } => types.get(name).and_then(|schema| {
            resolve_typed_schema_path_inner(schema, types, nested_path, depth + 1)
        }),
        armature_workflow::schema::Schema::Json | armature_workflow::schema::Schema::Map { .. } => {
            Some(armature_workflow::schema::Schema::Json)
        }
        armature_workflow::schema::Schema::Record { fields } => {
            let (segment, rest) = nested_path
                .split_once('.')
                .map_or((nested_path, ""), |(segment, rest)| (segment, rest));
            let field = fields.iter().find(|field| field.name == segment)?;
            resolve_typed_schema_path_inner(&field.schema, types, rest, depth + 1)
        }
        armature_workflow::schema::Schema::Union { variants } => {
            let mut resolved = variants
                .iter()
                .filter_map(|variant| {
                    resolve_typed_schema_path_inner(variant, types, nested_path, depth + 1)
                })
                .collect::<Vec<_>>();
            match resolved.len() {
                0 => None,
                1 => resolved.pop(),
                _ => Some(armature_workflow::schema::Schema::Union { variants: resolved }),
            }
        }
        _ => None,
    }
}

fn validate_expr_effects(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    ir: &armature_workflow::WorkflowIr,
    owner: &str,
    expr: &armature_workflow::expr::Expr,
    span: Option<&armature_workflow::SourceSpan>,
) {
    use armature_workflow::expr::Expr;

    match expr {
        Expr::Call { name, args } => {
            if let Some((capability, _)) = name.split_once('.') {
                if ir.capabilities.contains_key(capability) {
                    let request = AdapterEffectRequestShape {
                        effect_name: name.clone(),
                        category: armature_engine::effects::EffectCategory::SyncValue,
                    };
                    validate_effect_shape(diagnostics, registry, owner, request.as_ref(), span);
                }
            }

            for arg in args {
                validate_expr_effects(diagnostics, registry, ir, owner, arg, span);
            }
        }
        Expr::Eq { left, right }
        | Expr::Neq { left, right }
        | Expr::Lt { left, right }
        | Expr::Lte { left, right }
        | Expr::Gt { left, right }
        | Expr::Gte { left, right }
        | Expr::In { left, right } => {
            validate_expr_effects(diagnostics, registry, ir, owner, left, span);
            validate_expr_effects(diagnostics, registry, ir, owner, right, span);
        }
        Expr::And { exprs } | Expr::Or { exprs } => {
            for expr in exprs {
                validate_expr_effects(diagnostics, registry, ir, owner, expr, span);
            }
        }
        Expr::Not { expr } => {
            validate_expr_effects(diagnostics, registry, ir, owner, expr, span);
        }
        Expr::Object { fields } => {
            for expr in fields.values() {
                validate_expr_effects(diagnostics, registry, ir, owner, expr, span);
            }
        }
        Expr::List { items } => {
            for expr in items {
                validate_expr_effects(diagnostics, registry, ir, owner, expr, span);
            }
        }
        Expr::Literal { .. } | Expr::Path { .. } => {}
    }
}

fn validate_state_policy(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    policies: &[CapabilityPolicyDocument],
    ir: &armature_workflow::WorkflowIr,
    state_name: &str,
    state: &armature_workflow::ir::State,
) {
    let owner = format!("state `{state_name}`");

    for handler in &state.on {
        if let Some(guard) = &handler.guard {
            validate_expr_policy(
                diagnostics,
                registry,
                policies,
                ir,
                &owner,
                guard,
                handler.span.as_ref(),
            );
        }
        validate_steps_policy(diagnostics, registry, policies, ir, &owner, &handler.steps);
    }
    for transition in &state.always {
        if let Some(guard) = &transition.guard {
            validate_expr_policy(
                diagnostics,
                registry,
                policies,
                ir,
                &owner,
                guard,
                transition.span.as_ref(),
            );
        }
        validate_steps_policy(
            diagnostics,
            registry,
            policies,
            ir,
            &owner,
            &transition.steps,
        );
    }
    validate_steps_policy(diagnostics, registry, policies, ir, &owner, &state.entry);

    for (child_name, child) in &state.states {
        validate_state_policy(diagnostics, registry, policies, ir, child_name, child);
    }
}

fn validate_invariant_policy(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    policies: &[CapabilityPolicyDocument],
    ir: &armature_workflow::WorkflowIr,
) {
    for invariant in &ir.invariants {
        let armature_workflow::ir::Invariant::Expression { name, expr, span } = invariant else {
            continue;
        };
        let owner = format!("invariant `{name}`");
        validate_expr_policy(
            diagnostics,
            registry,
            policies,
            ir,
            &owner,
            expr,
            span.as_ref(),
        );
    }
}

fn validate_steps_policy(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    policies: &[CapabilityPolicyDocument],
    ir: &armature_workflow::WorkflowIr,
    owner: &str,
    steps: &[armature_workflow::ir::Step],
) {
    for step in steps {
        if let Some(request) = adapter_effect_request_shape(ir, step) {
            validate_effect_policy(
                diagnostics,
                registry,
                policies,
                owner,
                &request.effect_name,
                request.category,
                step.span.as_ref(),
            );
        }

        for value in step.args.values() {
            if let Ok(expr) = serde_json::from_value::<armature_workflow::expr::Expr>(value.clone())
            {
                validate_expr_policy(
                    diagnostics,
                    registry,
                    policies,
                    ir,
                    owner,
                    &expr,
                    step.span.as_ref(),
                );
            } else if let Ok(exprs) =
                serde_json::from_value::<Vec<armature_workflow::expr::Expr>>(value.clone())
            {
                for expr in &exprs {
                    validate_expr_policy(
                        diagnostics,
                        registry,
                        policies,
                        ir,
                        owner,
                        expr,
                        step.span.as_ref(),
                    );
                }
            }
        }

        for arm in &step.case_arms {
            validate_steps_policy(diagnostics, registry, policies, ir, owner, &arm.steps);
        }
    }
}

fn validate_expr_policy(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    policies: &[CapabilityPolicyDocument],
    ir: &armature_workflow::WorkflowIr,
    owner: &str,
    expr: &armature_workflow::expr::Expr,
    span: Option<&armature_workflow::SourceSpan>,
) {
    use armature_workflow::expr::Expr;

    match expr {
        Expr::Call { name, args } => {
            if let Some((capability, _)) = name.split_once('.') {
                if ir.capabilities.contains_key(capability) {
                    validate_effect_policy(
                        diagnostics,
                        registry,
                        policies,
                        owner,
                        name,
                        armature_engine::effects::EffectCategory::SyncValue,
                        span,
                    );
                }
            }

            for arg in args {
                validate_expr_policy(diagnostics, registry, policies, ir, owner, arg, span);
            }
        }
        Expr::Eq { left, right }
        | Expr::Neq { left, right }
        | Expr::Lt { left, right }
        | Expr::Lte { left, right }
        | Expr::Gt { left, right }
        | Expr::Gte { left, right }
        | Expr::In { left, right } => {
            validate_expr_policy(diagnostics, registry, policies, ir, owner, left, span);
            validate_expr_policy(diagnostics, registry, policies, ir, owner, right, span);
        }
        Expr::And { exprs } | Expr::Or { exprs } => {
            for expr in exprs {
                validate_expr_policy(diagnostics, registry, policies, ir, owner, expr, span);
            }
        }
        Expr::Not { expr } => {
            validate_expr_policy(diagnostics, registry, policies, ir, owner, expr, span);
        }
        Expr::Object { fields } => {
            for expr in fields.values() {
                validate_expr_policy(diagnostics, registry, policies, ir, owner, expr, span);
            }
        }
        Expr::List { items } => {
            for expr in items {
                validate_expr_policy(diagnostics, registry, policies, ir, owner, expr, span);
            }
        }
        Expr::Literal { .. } | Expr::Path { .. } => {}
    }
}

fn validate_effect_policy(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    policies: &[CapabilityPolicyDocument],
    owner: &str,
    effect_name: &str,
    category: armature_engine::effects::EffectCategory,
    span: Option<&armature_workflow::SourceSpan>,
) {
    let Some(registered) = registry.effect(effect_name) else {
        return;
    };

    for capability in &registered.effect.required_capabilities {
        validate_required_capability(
            diagnostics,
            policies,
            owner,
            effect_name,
            category,
            capability,
            span,
        );
    }
}

fn validate_effect_shape(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    registry: &EffectRegistry<'_>,
    owner: &str,
    request: AdapterEffectRequestShapeRef<'_>,
    span: Option<&armature_workflow::SourceSpan>,
) {
    match registry.effect(request.effect_name) {
        Some(registered) if registered.effect.category == request.category => {}
        Some(registered) => diagnostics.push(error_at(
            format!(
                "{owner} effect `{}` expects category `{:?}`, but adapter manifest declares `{:?}`",
                request.effect_name, request.category, registered.effect.category
            ),
            span,
        )),
        None => diagnostics.push(error_at(
            format!(
                "{owner} effect `{}` is not declared by any adapter manifest",
                request.effect_name
            ),
            span,
        )),
    }
}

struct AdapterEffectRequestShape {
    effect_name: String,
    category: armature_engine::effects::EffectCategory,
}

struct AdapterEffectRequestShapeRef<'a> {
    effect_name: &'a str,
    category: armature_engine::effects::EffectCategory,
}

impl AdapterEffectRequestShape {
    fn as_ref(&self) -> AdapterEffectRequestShapeRef<'_> {
        AdapterEffectRequestShapeRef {
            effect_name: &self.effect_name,
            category: self.category,
        }
    }
}

fn adapter_effect_request_shape(
    ir: &armature_workflow::WorkflowIr,
    step: &armature_workflow::ir::Step,
) -> Option<AdapterEffectRequestShape> {
    if is_native_agent_step(ir, step) {
        return None;
    }

    match step.effect.as_str() {
        "send" => Some(AdapterEffectRequestShape {
            effect_name: "send".to_string(),
            category: armature_engine::effects::EffectCategory::Message,
        }),
        "start" => Some(AdapterEffectRequestShape {
            effect_name: "start".to_string(),
            category: armature_engine::effects::EffectCategory::AsyncInvocation,
        }),
        "askHuman" => Some(AdapterEffectRequestShape {
            effect_name: "askHuman".to_string(),
            category: armature_engine::effects::EffectCategory::HumanObligation,
        }),
        "capability_call" => {
            let capability = step
                .args
                .get("capability")
                .and_then(|value| value.as_str())?;
            let operation = step
                .args
                .get("operation")
                .and_then(|value| value.as_str())
                .unwrap_or("call");
            Some(AdapterEffectRequestShape {
                effect_name: format!("{capability}.{operation}"),
                category: armature_engine::effects::EffectCategory::SyncValue,
            })
        }
        _ => None,
    }
}

fn is_native_agent_step(
    ir: &armature_workflow::WorkflowIr,
    step: &armature_workflow::ir::Step,
) -> bool {
    if !matches!(step.effect.as_str(), "start" | "send") {
        return false;
    }
    step.args
        .get("agent")
        .and_then(|value| value.as_str())
        .and_then(|agent| ir.agents.get(agent))
        .is_some_and(|agent| {
            !matches!(
                agent.target,
                armature_workflow::ir::AgentTarget::Adapter { .. }
            )
        })
}

fn validate_required_capability(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    policies: &[CapabilityPolicyDocument],
    owner: &str,
    effect_name: &str,
    category: armature_engine::effects::EffectCategory,
    capability: &str,
    span: Option<&armature_workflow::SourceSpan>,
) {
    if policies.iter().any(|policy| {
        policy
            .denied_capabilities
            .iter()
            .any(|denied| denied == capability)
    }) {
        diagnostics.push(error_at(
            format!("{owner} effect `{effect_name}` requires denied capability `{capability}`. Fix: remove `{capability}` from denied_capabilities only if this authority is intended, otherwise remove or replace the effect."),
            span,
        ));
        return;
    }

    if policies.iter().any(|policy| {
        policy
            .allowed_capabilities
            .iter()
            .any(|allowed| allowed == capability)
    }) {
        return;
    }

    let strictest_mode = policies
        .iter()
        .map(|policy| policy.mode)
        .max_by_key(policy_mode_rank)
        .unwrap_or(armature_workflow::policy::PolicyMode::Local);
    let severity = if unknown_capability_is_error(strictest_mode, category, capability) {
        armature_workflow::Severity::Error
    } else {
        armature_workflow::Severity::Warning
    };
    diagnostics.push(diagnostic_at(
        severity,
        format!(
            "{owner} effect `{effect_name}` requires capability `{capability}` that is not allowed by supplied policy. Fix: add `{capability}` to allowed_capabilities only if this authority is intended, otherwise remove or replace the effect."
        ),
        span,
    ));
}

fn policy_mode_rank(mode: &armature_workflow::policy::PolicyMode) -> u8 {
    match mode {
        armature_workflow::policy::PolicyMode::Local => 0,
        armature_workflow::policy::PolicyMode::Team => 1,
        armature_workflow::policy::PolicyMode::Enterprise => 2,
    }
}

fn unknown_capability_is_error(
    mode: armature_workflow::policy::PolicyMode,
    category: armature_engine::effects::EffectCategory,
    capability: &str,
) -> bool {
    match mode {
        armature_workflow::policy::PolicyMode::Local => false,
        armature_workflow::policy::PolicyMode::Enterprise => true,
        armature_workflow::policy::PolicyMode::Team => {
            matches!(
                category,
                armature_engine::effects::EffectCategory::AsyncInvocation
                    | armature_engine::effects::EffectCategory::HumanObligation
                    | armature_engine::effects::EffectCategory::Message
            ) || capability.contains(".write")
                || capability.starts_with("adapter.")
        }
    }
}

fn error(message: String) -> armature_workflow::Diagnostic {
    error_at(message, None)
}

fn error_at(
    message: String,
    span: Option<&armature_workflow::SourceSpan>,
) -> armature_workflow::Diagnostic {
    diagnostic_at(armature_workflow::Severity::Error, message, span)
}

fn diagnostic_at(
    severity: armature_workflow::Severity,
    message: String,
    span: Option<&armature_workflow::SourceSpan>,
) -> armature_workflow::Diagnostic {
    armature_workflow::Diagnostic {
        severity,
        message,
        span: span.cloned(),
    }
}

fn validate_policy_capability_list(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    policy_index: usize,
    field: &str,
    capabilities: &[String],
) {
    let mut seen = BTreeSet::new();
    for capability in capabilities {
        if capability.trim().is_empty() {
            diagnostics.push(error(format!(
                "policy document {policy_index} field `{field}` contains an empty capability"
            )));
        } else if has_invalid_token_characters(capability) {
            diagnostics.push(error(format!(
                "policy document {policy_index} field `{field}` capability `{capability}` contains whitespace or control characters"
            )));
        }
        if !seen.insert(capability) {
            diagnostics.push(error(format!(
                "policy document {policy_index} field `{field}` repeats capability `{capability}`"
            )));
        }
    }
}

fn validate_policy_token_list(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    policy_index: usize,
    field: &str,
    values: &[String],
) {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.trim().is_empty() {
            diagnostics.push(error(format!(
                "policy document {policy_index} field `{field}` contains an empty value"
            )));
        } else if has_invalid_token_characters(value) {
            diagnostics.push(error(format!(
                "policy document {policy_index} field `{field}` value `{value}` contains whitespace or control characters"
            )));
        }
        if !seen.insert(value) {
            diagnostics.push(error(format!(
                "policy document {policy_index} field `{field}` repeats value `{value}`"
            )));
        }
    }
}

fn validate_identifier(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    label: String,
    value: &str,
) {
    if value.trim().is_empty() {
        diagnostics.push(error(format!("{label} name must not be empty")));
    } else if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        diagnostics.push(error(format!(
            "{label} `{value}` must contain only ASCII letters, digits, or `_`"
        )));
    }
}

fn validate_effect_name(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    manifest_name: &str,
    effect_name: &str,
) {
    if effect_name.trim().is_empty() {
        diagnostics.push(error(format!(
            "adapter manifest `{manifest_name}` effect name must not be empty"
        )));
    } else if !effect_name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        diagnostics.push(error(format!(
            "adapter manifest `{manifest_name}` effect `{effect_name}` has an invalid name"
        )));
    }
}

fn validate_event_name(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    manifest_name: &str,
    event_name: &str,
) {
    if event_name.trim().is_empty() {
        diagnostics.push(error(format!(
            "adapter manifest `{manifest_name}` event name must not be empty"
        )));
    } else if !event_name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        diagnostics.push(error(format!(
            "adapter manifest `{manifest_name}` event `{event_name}` has an invalid name"
        )));
    }
}

fn validate_capability_list(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    manifest_name: &str,
    effect_name: &str,
    capabilities: &[String],
) {
    let mut seen = BTreeSet::new();
    for capability in capabilities {
        if capability.trim().is_empty() {
            diagnostics.push(error(format!(
                "adapter manifest `{manifest_name}` effect `{effect_name}` has an empty required capability"
            )));
        } else if has_invalid_token_characters(capability) {
            diagnostics.push(error(format!(
                "adapter manifest `{manifest_name}` effect `{effect_name}` required capability `{capability}` contains whitespace or control characters"
            )));
        }
        if !seen.insert(capability) {
            diagnostics.push(error(format!(
                "adapter manifest `{manifest_name}` effect `{effect_name}` repeats required capability `{capability}`"
            )));
        }
    }
}

fn validate_token_list(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    manifest_name: &str,
    effect_name: &str,
    label: &str,
    values: &[String],
) {
    let mut seen = BTreeSet::new();
    for value in values {
        if value.trim().is_empty() {
            diagnostics.push(error(format!(
                "adapter manifest `{manifest_name}` effect `{effect_name}` has an empty {label}"
            )));
        } else if has_invalid_token_characters(value) {
            diagnostics.push(error(format!(
                "adapter manifest `{manifest_name}` effect `{effect_name}` {label} `{value}` contains whitespace or control characters"
            )));
        }
        if !seen.insert(value) {
            diagnostics.push(error(format!(
                "adapter manifest `{manifest_name}` effect `{effect_name}` repeats {label} `{value}`"
            )));
        }
    }
}

fn has_invalid_token_characters(token: &str) -> bool {
    token.chars().any(char::is_whitespace) || token.chars().any(char::is_control)
}

fn validate_schema_refs(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    manifest_name: &str,
    location: &str,
    schema: &armature_workflow::schema::Schema,
    types: &BTreeMap<String, armature_workflow::schema::Schema>,
) {
    match schema {
        armature_workflow::schema::Schema::Ref { name } => {
            if !types.contains_key(name) {
                diagnostics.push(error(format!(
                    "adapter manifest `{manifest_name}` {location} references unknown type `{name}`"
                )));
            }
        }
        armature_workflow::schema::Schema::Optional { inner }
        | armature_workflow::schema::Schema::List { inner }
        | armature_workflow::schema::Schema::Set { inner } => {
            validate_schema_refs(diagnostics, manifest_name, location, inner, types);
        }
        armature_workflow::schema::Schema::Map { key, value } => {
            validate_schema_refs(diagnostics, manifest_name, location, key, types);
            validate_schema_refs(diagnostics, manifest_name, location, value, types);
        }
        armature_workflow::schema::Schema::Union { variants } => {
            for variant in variants {
                validate_schema_refs(diagnostics, manifest_name, location, variant, types);
            }
        }
        armature_workflow::schema::Schema::Record { fields } => {
            for field in fields {
                validate_schema_refs(diagnostics, manifest_name, location, &field.schema, types);
            }
        }
        armature_workflow::schema::Schema::String
        | armature_workflow::schema::Schema::Int
        | armature_workflow::schema::Schema::Float
        | armature_workflow::schema::Schema::Boolean
        | armature_workflow::schema::Schema::Null
        | armature_workflow::schema::Schema::Time
        | armature_workflow::schema::Schema::Duration
        | armature_workflow::schema::Schema::Agent
        | armature_workflow::schema::Schema::Literal { .. }
        | armature_workflow::schema::Schema::Enum { .. }
        | armature_workflow::schema::Schema::Json => {}
    }
}

fn validate_schema_uniqueness(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    manifest_name: &str,
    location: &str,
    schema: &armature_workflow::schema::Schema,
) {
    match schema {
        armature_workflow::schema::Schema::Enum { values } => {
            let mut seen = BTreeSet::new();
            for value in values {
                if !seen.insert(value) {
                    diagnostics.push(error(format!(
                        "adapter manifest `{manifest_name}` {location} repeats enum value `{value}`"
                    )));
                }
            }
        }
        armature_workflow::schema::Schema::Record { fields } => {
            let mut seen = BTreeSet::new();
            for field in fields {
                if !seen.insert(&field.name) {
                    diagnostics.push(error(format!(
                        "adapter manifest `{manifest_name}` {location} repeats record field `{}`",
                        field.name
                    )));
                }
                validate_schema_uniqueness(diagnostics, manifest_name, location, &field.schema);
            }
        }
        armature_workflow::schema::Schema::Optional { inner }
        | armature_workflow::schema::Schema::List { inner }
        | armature_workflow::schema::Schema::Set { inner } => {
            validate_schema_uniqueness(diagnostics, manifest_name, location, inner);
        }
        armature_workflow::schema::Schema::Map { key, value } => {
            validate_schema_uniqueness(diagnostics, manifest_name, location, key);
            validate_schema_uniqueness(diagnostics, manifest_name, location, value);
        }
        armature_workflow::schema::Schema::Union { variants } => {
            for variant in variants {
                validate_schema_uniqueness(diagnostics, manifest_name, location, variant);
            }
        }
        armature_workflow::schema::Schema::String
        | armature_workflow::schema::Schema::Int
        | armature_workflow::schema::Schema::Float
        | armature_workflow::schema::Schema::Boolean
        | armature_workflow::schema::Schema::Null
        | armature_workflow::schema::Schema::Time
        | armature_workflow::schema::Schema::Duration
        | armature_workflow::schema::Schema::Agent
        | armature_workflow::schema::Schema::Literal { .. }
        | armature_workflow::schema::Schema::Ref { .. }
        | armature_workflow::schema::Schema::Json => {}
    }
}

fn validate_map_key_schemas(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    manifest_name: &str,
    location: &str,
    schema: &armature_workflow::schema::Schema,
    types: &BTreeMap<String, armature_workflow::schema::Schema>,
) {
    validate_map_key_schemas_inner(diagnostics, manifest_name, location, schema, types, 0);
}

fn validate_map_key_schemas_inner(
    diagnostics: &mut Vec<armature_workflow::Diagnostic>,
    manifest_name: &str,
    location: &str,
    schema: &armature_workflow::schema::Schema,
    types: &BTreeMap<String, armature_workflow::schema::Schema>,
    depth: usize,
) {
    if depth > 16 {
        return;
    }

    match schema {
        armature_workflow::schema::Schema::Optional { inner }
        | armature_workflow::schema::Schema::List { inner }
        | armature_workflow::schema::Schema::Set { inner } => {
            validate_map_key_schemas_inner(
                diagnostics,
                manifest_name,
                location,
                inner,
                types,
                depth + 1,
            );
        }
        armature_workflow::schema::Schema::Map { key, value } => {
            if !map_key_schema_is_string_compatible(key, types, depth + 1) {
                diagnostics.push(error(format!(
                    "adapter manifest `{manifest_name}` {location} declares map key type `{}`; map keys must be string-compatible",
                    adapter_schema_kind(key)
                )));
            }
            validate_map_key_schemas_inner(
                diagnostics,
                manifest_name,
                location,
                value,
                types,
                depth + 1,
            );
        }
        armature_workflow::schema::Schema::Union { variants } => {
            for variant in variants {
                validate_map_key_schemas_inner(
                    diagnostics,
                    manifest_name,
                    location,
                    variant,
                    types,
                    depth + 1,
                );
            }
        }
        armature_workflow::schema::Schema::Record { fields } => {
            for field in fields {
                validate_map_key_schemas_inner(
                    diagnostics,
                    manifest_name,
                    location,
                    &field.schema,
                    types,
                    depth + 1,
                );
            }
        }
        armature_workflow::schema::Schema::Ref { name } => {
            if let Some(schema) = types.get(name) {
                validate_map_key_schemas_inner(
                    diagnostics,
                    manifest_name,
                    location,
                    schema,
                    types,
                    depth + 1,
                );
            }
        }
        armature_workflow::schema::Schema::String
        | armature_workflow::schema::Schema::Int
        | armature_workflow::schema::Schema::Float
        | armature_workflow::schema::Schema::Boolean
        | armature_workflow::schema::Schema::Null
        | armature_workflow::schema::Schema::Time
        | armature_workflow::schema::Schema::Duration
        | armature_workflow::schema::Schema::Agent
        | armature_workflow::schema::Schema::Literal { .. }
        | armature_workflow::schema::Schema::Enum { .. }
        | armature_workflow::schema::Schema::Json => {}
    }
}

fn map_key_schema_is_string_compatible(
    schema: &armature_workflow::schema::Schema,
    types: &BTreeMap<String, armature_workflow::schema::Schema>,
    depth: usize,
) -> bool {
    if depth > 16 {
        return false;
    }

    match schema {
        armature_workflow::schema::Schema::String
        | armature_workflow::schema::Schema::Enum { .. } => true,
        armature_workflow::schema::Schema::Literal { value } => value.is_string(),
        armature_workflow::schema::Schema::Union { variants } => {
            !variants.is_empty()
                && variants
                    .iter()
                    .all(|variant| map_key_schema_is_string_compatible(variant, types, depth + 1))
        }
        armature_workflow::schema::Schema::Ref { name } => types
            .get(name)
            .is_none_or(|schema| map_key_schema_is_string_compatible(schema, types, depth + 1)),
        armature_workflow::schema::Schema::Int
        | armature_workflow::schema::Schema::Float
        | armature_workflow::schema::Schema::Boolean
        | armature_workflow::schema::Schema::Null
        | armature_workflow::schema::Schema::Time
        | armature_workflow::schema::Schema::Duration
        | armature_workflow::schema::Schema::Agent
        | armature_workflow::schema::Schema::Optional { .. }
        | armature_workflow::schema::Schema::List { .. }
        | armature_workflow::schema::Schema::Set { .. }
        | armature_workflow::schema::Schema::Map { .. }
        | armature_workflow::schema::Schema::Record { .. }
        | armature_workflow::schema::Schema::Json => false,
    }
}

fn adapter_schema_kind(schema: &armature_workflow::schema::Schema) -> &'static str {
    match schema {
        armature_workflow::schema::Schema::String => "string",
        armature_workflow::schema::Schema::Int => "int",
        armature_workflow::schema::Schema::Float => "float",
        armature_workflow::schema::Schema::Boolean => "bool",
        armature_workflow::schema::Schema::Null => "null",
        armature_workflow::schema::Schema::Time => "time",
        armature_workflow::schema::Schema::Duration => "duration",
        armature_workflow::schema::Schema::Agent => "agent",
        armature_workflow::schema::Schema::Literal { .. } => "literal",
        armature_workflow::schema::Schema::Enum { .. } => "enum",
        armature_workflow::schema::Schema::Optional { .. } => "optional",
        armature_workflow::schema::Schema::List { .. } => "list",
        armature_workflow::schema::Schema::Set { .. } => "set",
        armature_workflow::schema::Schema::Map { .. } => "map",
        armature_workflow::schema::Schema::Union { .. } => "union",
        armature_workflow::schema::Schema::Record { .. } => "record",
        armature_workflow::schema::Schema::Ref { .. } => "ref",
        armature_workflow::schema::Schema::Json => "json",
    }
}

fn schemas_equivalent(
    left: &armature_workflow::schema::Schema,
    left_types: &BTreeMap<String, armature_workflow::schema::Schema>,
    right: &armature_workflow::schema::Schema,
    right_types: &BTreeMap<String, armature_workflow::schema::Schema>,
    depth: usize,
) -> bool {
    if depth > 16 {
        return false;
    }

    let left = resolve_schema_ref(left, left_types);
    let right = resolve_schema_ref(right, right_types);
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };

    use armature_workflow::schema::Schema;
    match (left, right) {
        (Schema::String, Schema::String)
        | (Schema::Int, Schema::Int)
        | (Schema::Float, Schema::Float)
        | (Schema::Boolean, Schema::Boolean)
        | (Schema::Null, Schema::Null)
        | (Schema::Time, Schema::Time)
        | (Schema::Duration, Schema::Duration)
        | (Schema::Agent, Schema::Agent)
        | (Schema::Json, Schema::Json) => true,
        (Schema::Literal { value: left }, Schema::Literal { value: right }) => left == right,
        (Schema::Enum { values: left }, Schema::Enum { values: right }) => left == right,
        (Schema::Optional { inner: left }, Schema::Optional { inner: right })
        | (Schema::List { inner: left }, Schema::List { inner: right })
        | (Schema::Set { inner: left }, Schema::Set { inner: right }) => {
            schemas_equivalent(left, left_types, right, right_types, depth + 1)
        }
        (
            Schema::Map {
                key: left_key,
                value: left_value,
            },
            Schema::Map {
                key: right_key,
                value: right_value,
            },
        ) => {
            schemas_equivalent(left_key, left_types, right_key, right_types, depth + 1)
                && schemas_equivalent(left_value, left_types, right_value, right_types, depth + 1)
        }
        (Schema::Union { variants: left }, Schema::Union { variants: right }) => {
            left.len() == right.len()
                && left.iter().all(|left| {
                    right.iter().any(|right| {
                        schemas_equivalent(left, left_types, right, right_types, depth + 1)
                    })
                })
        }
        (Schema::Record { fields: left }, Schema::Record { fields: right }) => {
            left.len() == right.len()
                && left.iter().all(|left| {
                    right.iter().any(|right| {
                        left.name == right.name
                            && schemas_equivalent(
                                &left.schema,
                                left_types,
                                &right.schema,
                                right_types,
                                depth + 1,
                            )
                    })
                })
        }
        _ => false,
    }
}

fn resolve_schema_ref<'a>(
    schema: &'a armature_workflow::schema::Schema,
    types: &'a BTreeMap<String, armature_workflow::schema::Schema>,
) -> Option<&'a armature_workflow::schema::Schema> {
    match schema {
        armature_workflow::schema::Schema::Ref { name } => types.get(name),
        schema => Some(schema),
    }
}

fn adapter_schema_accepts_schema(
    expected: &armature_workflow::schema::Schema,
    expected_types: &BTreeMap<String, armature_workflow::schema::Schema>,
    actual: &armature_workflow::schema::Schema,
    actual_types: &BTreeMap<String, armature_workflow::schema::Schema>,
    depth: usize,
) -> bool {
    if depth > 16 {
        return true;
    }

    use armature_workflow::schema::Schema;

    let expected = resolve_schema_ref(expected, expected_types);
    let actual = resolve_schema_ref(actual, actual_types);
    let (Some(expected), Some(actual)) = (expected, actual) else {
        return false;
    };

    match (expected, actual) {
        (Schema::Json, _) | (_, Schema::Json) => true,
        (
            Schema::Optional {
                inner: expected_inner,
            },
            Schema::Optional {
                inner: actual_inner,
            },
        ) => adapter_schema_accepts_schema(
            expected_inner,
            expected_types,
            actual_inner,
            actual_types,
            depth + 1,
        ),
        (Schema::Optional { .. }, Schema::Null) => true,
        (Schema::Optional { .. }, Schema::Literal { value }) if value.is_null() => true,
        (Schema::Optional { inner }, _) => {
            adapter_schema_accepts_schema(inner, expected_types, actual, actual_types, depth + 1)
        }
        (_, Schema::Optional { .. }) => false,
        (Schema::Union { variants }, _) => variants.iter().any(|variant| {
            adapter_schema_accepts_schema(variant, expected_types, actual, actual_types, depth + 1)
        }),
        (_, Schema::Union { variants }) => variants.iter().all(|variant| {
            adapter_schema_accepts_schema(
                expected,
                expected_types,
                variant,
                actual_types,
                depth + 1,
            )
        }),
        (Schema::Literal { value: expected }, Schema::Literal { value: actual }) => {
            expected == actual
        }
        (
            Schema::String | Schema::Time | Schema::Duration | Schema::Agent,
            Schema::Literal { value },
        ) => value.is_string(),
        (Schema::Int, Schema::Literal { value }) => {
            value.as_i64().is_some() || value.as_u64().is_some()
        }
        (Schema::Float, Schema::Literal { value }) => value.is_number(),
        (Schema::Boolean, Schema::Literal { value }) => value.is_boolean(),
        (Schema::Null, Schema::Literal { value }) => value.is_null(),
        (Schema::Enum { values }, Schema::Literal { value }) => value
            .as_str()
            .is_some_and(|actual| values.iter().any(|expected| expected == actual)),
        (Schema::Float, Schema::Int) => true,
        (Schema::Enum { values: expected }, Schema::Enum { values: actual }) => {
            actual.iter().all(|value| expected.contains(value))
        }
        (Schema::List { inner: expected }, Schema::List { inner: actual })
        | (Schema::Set { inner: expected }, Schema::Set { inner: actual }) => {
            adapter_schema_accepts_schema(expected, expected_types, actual, actual_types, depth + 1)
        }
        (
            Schema::Map {
                key: expected_key,
                value: expected_value,
            },
            Schema::Map {
                key: actual_key,
                value: actual_value,
            },
        ) => {
            adapter_schema_accepts_schema(
                expected_key,
                expected_types,
                actual_key,
                actual_types,
                depth + 1,
            ) && adapter_schema_accepts_schema(
                expected_value,
                expected_types,
                actual_value,
                actual_types,
                depth + 1,
            )
        }
        (Schema::Record { fields: expected }, Schema::Record { fields: actual }) => {
            actual.iter().all(|actual_field| {
                expected
                    .iter()
                    .any(|expected_field| expected_field.name == actual_field.name)
            }) && expected.iter().all(|expected_field| {
                let Some(actual_field) = actual
                    .iter()
                    .find(|actual_field| actual_field.name == expected_field.name)
                else {
                    return adapter_schema_allows_absent(
                        &expected_field.schema,
                        expected_types,
                        depth + 1,
                    );
                };
                adapter_schema_accepts_schema(
                    &expected_field.schema,
                    expected_types,
                    &actual_field.schema,
                    actual_types,
                    depth + 1,
                )
            })
        }
        _ => adapter_schema_kind(expected) == adapter_schema_kind(actual),
    }
}

fn adapter_schema_allows_absent(
    schema: &armature_workflow::schema::Schema,
    types: &BTreeMap<String, armature_workflow::schema::Schema>,
    depth: usize,
) -> bool {
    if depth > 16 {
        return false;
    }

    match resolve_schema_ref(schema, types) {
        Some(armature_workflow::schema::Schema::Optional { .. }) => true,
        Some(armature_workflow::schema::Schema::Union { variants }) => variants
            .iter()
            .any(|variant| adapter_schema_allows_absent(variant, types, depth + 1)),
        _ => false,
    }
}

#[derive(Debug, Clone, Default)]
pub struct ManifestEffectDispatcher {
    manifests: Vec<AdapterManifest>,
    policies: Vec<CapabilityPolicyDocument>,
    fake_outputs: BTreeMap<String, serde_json::Value>,
    json_plan_file: Option<PathBuf>,
    human_review_file: Option<PathBuf>,
    agent_file: Option<PathBuf>,
}

impl ManifestEffectDispatcher {
    pub fn new(manifest: AdapterManifest) -> Self {
        Self {
            manifests: vec![manifest],
            policies: Vec::new(),
            fake_outputs: BTreeMap::new(),
            json_plan_file: None,
            human_review_file: None,
            agent_file: None,
        }
    }

    pub fn from_manifests(manifests: Vec<AdapterManifest>) -> Self {
        Self {
            manifests,
            policies: Vec::new(),
            fake_outputs: BTreeMap::new(),
            json_plan_file: None,
            human_review_file: None,
            agent_file: None,
        }
    }

    pub fn with_policies(mut self, policies: Vec<CapabilityPolicyDocument>) -> Self {
        self.policies = policies;
        self
    }

    pub fn with_fake_output(
        mut self,
        effect: impl Into<String>,
        output: serde_json::Value,
    ) -> Self {
        self.fake_outputs.insert(effect.into(), output);
        self
    }

    pub fn with_json_plan_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.json_plan_file = Some(path.into());
        self
    }

    pub fn with_human_review_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.human_review_file = Some(path.into());
        self
    }

    pub fn with_agent_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.agent_file = Some(path.into());
        self
    }

    fn find_effect(&self, effect: &str) -> Option<(&AdapterManifest, &AdapterEffect)> {
        self.manifests.iter().find_map(|manifest| {
            manifest
                .effects
                .get(effect)
                .map(|adapter_effect| (manifest, adapter_effect))
        })
    }
}

impl armature_engine::effects::EffectDispatcher for ManifestEffectDispatcher {
    fn dispatch(
        &mut self,
        request: armature_engine::effects::EffectRequest,
    ) -> Result<armature_engine::effects::EffectOutcome, armature_engine::effects::EffectError>
    {
        let Some((manifest, effect)) = self.find_effect(&request.effect) else {
            if matches!(
                request.category,
                armature_engine::effects::EffectCategory::Event
                    | armature_engine::effects::EffectCategory::Timer
                    | armature_engine::effects::EffectCategory::Terminal
            ) {
                let required_capabilities = request.required_capabilities.clone();
                return Ok(successful_outcome(request, None, required_capabilities));
            }

            return Err(armature_engine::effects::EffectError::Unsupported(format!(
                "effect `{}` is not declared by any adapter manifest",
                request.effect
            )));
        };

        if effect.category != request.category {
            return Err(armature_engine::effects::EffectError::Unsupported(format!(
                "effect `{}` has category `{:?}` in adapter `{}`, but request used `{:?}`",
                request.effect, effect.category, manifest.name, request.category
            )));
        }

        if !effect
            .input
            .accepts_json_with_types(&request.args, &manifest.types)
        {
            return Err(armature_engine::effects::EffectError::Unsupported(format!(
                "effect `{}` input does not match adapter `{}` manifest schema",
                request.effect, manifest.name
            )));
        }

        enforce_runtime_policy(&self.policies, &request, effect)?;

        if let Some(path) = &self.json_plan_file {
            if let Some(output) = dispatch_json_plan_effect(path, &request)? {
                if !effect
                    .output
                    .accepts_json_with_types(&output, &manifest.types)
                {
                    return Err(armature_engine::effects::EffectError::Unsupported(format!(
                        "effect `{}` JSON plan output does not match adapter `{}` manifest schema",
                        request.effect, manifest.name
                    )));
                }

                return Ok(successful_outcome(
                    request,
                    Some(output),
                    effect.required_capabilities.clone(),
                ));
            }
        }

        if let Some(path) = &self.human_review_file {
            if let Some(output) = dispatch_human_review_effect(path, &request)? {
                if !effect
                    .output
                    .accepts_json_with_types(&output, &manifest.types)
                {
                    return Err(armature_engine::effects::EffectError::Unsupported(format!(
                        "effect `{}` human review output does not match adapter `{}` manifest schema",
                        request.effect, manifest.name
                    )));
                }

                return Ok(successful_outcome(
                    request,
                    Some(output),
                    effect.required_capabilities.clone(),
                ));
            }
        }

        if let Some(path) = &self.agent_file {
            if let Some(output) = dispatch_agent_file_effect(path, &request)? {
                if !effect
                    .output
                    .accepts_json_with_types(&output, &manifest.types)
                {
                    return Err(armature_engine::effects::EffectError::Unsupported(format!(
                        "effect `{}` agent file output does not match adapter `{}` manifest schema",
                        request.effect, manifest.name
                    )));
                }

                return Ok(successful_outcome(
                    request,
                    Some(output),
                    effect.required_capabilities.clone(),
                ));
            }
        }

        let output = self.fake_outputs.get(&request.effect).cloned();
        if let Some(output) = &output {
            if !effect
                .output
                .accepts_json_with_types(output, &manifest.types)
            {
                return Err(armature_engine::effects::EffectError::Unsupported(format!(
                    "effect `{}` fake output does not match adapter `{}` manifest schema",
                    request.effect, manifest.name
                )));
            }
        }

        Ok(successful_outcome(
            request,
            output,
            effect.required_capabilities.clone(),
        ))
    }
}

fn enforce_runtime_policy(
    policies: &[CapabilityPolicyDocument],
    request: &armature_engine::effects::EffectRequest,
    effect: &AdapterEffect,
) -> Result<(), armature_engine::effects::EffectError> {
    for capability in &effect.required_capabilities {
        if let Err(message) =
            runtime_policy_allows(policies, request.category, &request.effect, capability)
        {
            return Err(armature_engine::effects::EffectError::CapabilityDenied {
                message,
                required_capabilities: effect.required_capabilities.clone(),
            });
        }
    }
    Ok(())
}

fn dispatch_json_plan_effect(
    path: &Path,
    request: &armature_engine::effects::EffectRequest,
) -> Result<Option<serde_json::Value>, armature_engine::effects::EffectError> {
    match request.effect.as_str() {
        "plan.snapshot" => {
            let contents = std::fs::read_to_string(path).map_err(|error| {
                armature_engine::effects::EffectError::Unsupported(format!(
                    "failed to read JSON plan file `{}`: {error}",
                    path.display()
                ))
            })?;
            Ok(Some(serde_json::Value::String(contents)))
        }
        "plan.unfinishedItems" => {
            let plan = read_json_plan(path)?;
            Ok(Some(serde_json::json!(count_unfinished_items(&plan)?)))
        }
        "plan.nextReadyItem" => {
            let plan = read_json_plan(path)?;
            Ok(Some(
                next_ready_item(&plan)?.unwrap_or(serde_json::Value::Null),
            ))
        }
        "plan.markReadyForQuality" => update_json_plan_status(
            path,
            request,
            PlanStatusUpdate {
                status: "ready_for_quality",
                reason_arg: None,
            },
        )
        .map(Some),
        "plan.markDone" => update_json_plan_status(
            path,
            request,
            PlanStatusUpdate {
                status: "done",
                reason_arg: None,
            },
        )
        .map(Some),
        "plan.markBlocked" => update_json_plan_status(
            path,
            request,
            PlanStatusUpdate {
                status: "blocked",
                reason_arg: Some(1),
            },
        )
        .map(Some),
        _ => Ok(None),
    }
}

fn dispatch_human_review_effect(
    path: &Path,
    request: &armature_engine::effects::EffectRequest,
) -> Result<Option<serde_json::Value>, armature_engine::effects::EffectError> {
    if request.effect != "askHuman" {
        return Ok(None);
    }

    let reason = request
        .args
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            armature_engine::effects::EffectError::Unsupported(
                "askHuman requires a string `reason` argument".to_string(),
            )
        })?;
    let review_id = format!("review-{}", request.effect_id);
    let obligation = serde_json::json!({
        "id": review_id,
        "status": "open",
        "reason": reason,
        "workflow_id": request.workflow_id,
        "effect_id": request.effect_id,
        "transition_id": request.transition_id,
        "idempotency_key": request.idempotency_key,
    });

    append_human_review_obligation(path, obligation.clone())?;
    Ok(Some(obligation))
}

fn append_human_review_obligation(
    path: &Path,
    obligation: serde_json::Value,
) -> Result<(), armature_engine::effects::EffectError> {
    let _lock = JsonFileLock::acquire(path)?;
    let mut document = if path.exists() {
        let contents = std::fs::read_to_string(path).map_err(|error| {
            armature_engine::effects::EffectError::Unsupported(format!(
                "failed to read human review file `{}`: {error}",
                path.display()
            ))
        })?;
        serde_json::from_str(&contents).map_err(|error| {
            armature_engine::effects::EffectError::Unsupported(format!(
                "human review file `{}` is not valid JSON: {error}",
                path.display()
            ))
        })?
    } else {
        serde_json::json!({ "reviews": [] })
    };

    let Some(document_object) = document.as_object_mut() else {
        return Err(armature_engine::effects::EffectError::Unsupported(
            "human review file root must be an object".to_string(),
        ));
    };
    let reviews = document_object
        .entry("reviews")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let Some(reviews) = reviews.as_array_mut() else {
        return Err(armature_engine::effects::EffectError::Unsupported(
            "human review file field `reviews` must be an array".to_string(),
        ));
    };

    let idempotency_key = obligation
        .get("idempotency_key")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    if let Some(idempotency_key) = idempotency_key {
        if reviews.iter().any(|review| {
            review
                .get("idempotency_key")
                .and_then(serde_json::Value::as_str)
                == Some(idempotency_key.as_str())
        }) {
            write_json_document(path, &document)?;
            return Ok(());
        }
    }

    reviews.push(obligation);
    write_json_document(path, &document)
}

fn dispatch_agent_file_effect(
    path: &Path,
    request: &armature_engine::effects::EffectRequest,
) -> Result<Option<serde_json::Value>, armature_engine::effects::EffectError> {
    match request.effect.as_str() {
        "start" => {
            let input = request
                .args
                .get("input")
                .cloned()
                .unwrap_or_else(|| request.args.clone());
            let invocation = serde_json::json!({
                "id": request.idempotency_key,
                "status": "started",
                "agent": request.target,
                "workflow_id": request.workflow_id,
                "effect_id": request.effect_id,
                "transition_id": request.transition_id,
                "input": input,
            });
            append_agent_record(path, "invocations", invocation.clone())?;
            Ok(Some(invocation))
        }
        "send" => {
            let message = serde_json::json!({
                "id": request.idempotency_key,
                "status": "sent",
                "agent": request.target,
                "workflow_id": request.workflow_id,
                "effect_id": request.effect_id,
                "transition_id": request.transition_id,
                "message": request.args.get("message").cloned().unwrap_or(serde_json::Value::Null),
            });
            append_agent_record(path, "messages", message.clone())?;
            Ok(Some(message))
        }
        _ => Ok(None),
    }
}

fn append_agent_record(
    path: &Path,
    collection: &str,
    record: serde_json::Value,
) -> Result<(), armature_engine::effects::EffectError> {
    let _lock = JsonFileLock::acquire(path)?;
    let mut document = if path.exists() {
        let contents = std::fs::read_to_string(path).map_err(|error| {
            armature_engine::effects::EffectError::Unsupported(format!(
                "failed to read agent file `{}`: {error}",
                path.display()
            ))
        })?;
        serde_json::from_str(&contents).map_err(|error| {
            armature_engine::effects::EffectError::Unsupported(format!(
                "agent file `{}` is not valid JSON: {error}",
                path.display()
            ))
        })?
    } else {
        serde_json::json!({
            "invocations": [],
            "messages": []
        })
    };

    let Some(document_object) = document.as_object_mut() else {
        return Err(armature_engine::effects::EffectError::Unsupported(
            "agent file root must be an object".to_string(),
        ));
    };
    let records = document_object
        .entry(collection)
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let Some(records) = records.as_array_mut() else {
        return Err(armature_engine::effects::EffectError::Unsupported(format!(
            "agent file field `{collection}` must be an array"
        )));
    };

    let id = record
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    if let Some(id) = id {
        if records.iter().any(|existing| {
            existing.get("id").and_then(serde_json::Value::as_str) == Some(id.as_str())
        }) {
            write_json_document(path, &document)?;
            return Ok(());
        }
    }

    records.push(record);
    write_json_document(path, &document)
}

struct PlanStatusUpdate<'a> {
    status: &'a str,
    reason_arg: Option<usize>,
}

fn update_json_plan_status(
    path: &Path,
    request: &armature_engine::effects::EffectRequest,
    update: PlanStatusUpdate<'_>,
) -> Result<serde_json::Value, armature_engine::effects::EffectError> {
    let _lock = JsonFileLock::acquire(path)?;
    let mut plan = read_json_plan(path)?;
    let work_item_id = plan_call_string_arg(request, 0)?;
    let reason = update
        .reason_arg
        .map(|index| plan_call_string_arg(request, index))
        .transpose()?;

    let updated_existing_task =
        update_task_array_status(&mut plan, &work_item_id, update.status, reason.as_deref())?;
    if !updated_existing_task {
        update_status_map(&mut plan, &work_item_id, update.status, reason.as_deref())?;
    }

    write_json_document(path, &plan)?;
    Ok(serde_json::json!({
        "workItemId": work_item_id,
        "status": update.status,
        "updated": true
    }))
}

fn count_unfinished_items(
    plan: &serde_json::Value,
) -> Result<i64, armature_engine::effects::EffectError> {
    if let Some(tasks) = plan.get("tasks") {
        let Some(tasks) = tasks.as_array() else {
            return Err(armature_engine::effects::EffectError::Unsupported(
                "JSON plan field `tasks` must be an array".to_string(),
            ));
        };
        return Ok(tasks
            .iter()
            .filter(|task| task_status(task) != Some("done"))
            .count() as i64);
    }

    if let Some(statuses) = plan.get("statuses") {
        let Some(statuses) = statuses.as_object() else {
            return Err(armature_engine::effects::EffectError::Unsupported(
                "JSON plan field `statuses` must be an object".to_string(),
            ));
        };
        return Ok(statuses
            .values()
            .filter(|status| task_status(status) != Some("done"))
            .count() as i64);
    }

    Ok(0)
}

fn next_ready_item(
    plan: &serde_json::Value,
) -> Result<Option<serde_json::Value>, armature_engine::effects::EffectError> {
    let Some(tasks) = plan.get("tasks") else {
        return Ok(None);
    };
    let Some(tasks) = tasks.as_array() else {
        return Err(armature_engine::effects::EffectError::Unsupported(
            "JSON plan field `tasks` must be an array".to_string(),
        ));
    };
    Ok(tasks
        .iter()
        .find(|task| {
            matches!(
                task_status(task),
                None | Some("todo") | Some("ready") | Some("ready_for_implementation")
            )
        })
        .cloned())
}

fn task_status(task: &serde_json::Value) -> Option<&str> {
    task.get("status")
        .and_then(serde_json::Value::as_str)
        .or_else(|| task.as_str())
}

struct JsonFileLock {
    path: PathBuf,
}

impl JsonFileLock {
    fn acquire(path: &Path) -> Result<Self, armature_engine::effects::EffectError> {
        let lock_path = json_file_lock_path(path);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(_) => return Ok(Self { path: lock_path }),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        return Err(armature_engine::effects::EffectError::Unsupported(format!(
                            "timed out waiting for JSON plan lock `{}`",
                            lock_path.display()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(armature_engine::effects::EffectError::Unsupported(format!(
                        "failed to acquire JSON plan lock `{}`: {error}",
                        lock_path.display()
                    )));
                }
            }
        }
    }
}

impl Drop for JsonFileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn json_file_lock_path(path: &Path) -> PathBuf {
    let mut path = path.as_os_str().to_os_string();
    path.push(".lock");
    PathBuf::from(path)
}

fn read_json_plan(path: &Path) -> Result<serde_json::Value, armature_engine::effects::EffectError> {
    let contents = std::fs::read_to_string(path).map_err(|error| {
        armature_engine::effects::EffectError::Unsupported(format!(
            "failed to read JSON plan file `{}`: {error}",
            path.display()
        ))
    })?;
    serde_json::from_str(&contents).map_err(|error| {
        armature_engine::effects::EffectError::Unsupported(format!(
            "JSON plan file `{}` is not valid JSON: {error}",
            path.display()
        ))
    })
}

fn read_json_document_or_default(
    path: &Path,
    default: serde_json::Value,
) -> Result<serde_json::Value, armature_engine::effects::EffectError> {
    if !path.exists() {
        return Ok(default);
    }

    let contents = std::fs::read_to_string(path).map_err(|error| {
        armature_engine::effects::EffectError::Unsupported(format!(
            "failed to read JSON file `{}`: {error}",
            path.display()
        ))
    })?;
    serde_json::from_str(&contents).map_err(|error| {
        armature_engine::effects::EffectError::Unsupported(format!(
            "JSON file `{}` is not valid JSON: {error}",
            path.display()
        ))
    })
}

fn write_json_document(
    path: &Path,
    plan: &serde_json::Value,
) -> Result<(), armature_engine::effects::EffectError> {
    let contents = serde_json::to_string_pretty(plan).map_err(|error| {
        armature_engine::effects::EffectError::Unsupported(format!(
            "failed to serialize JSON plan update for `{}`: {error}",
            path.display()
        ))
    })?;
    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, format!("{contents}\n")).map_err(|error| {
        armature_engine::effects::EffectError::Unsupported(format!(
            "failed to write temporary JSON plan file `{}`: {error}",
            tmp_path.display()
        ))
    })?;
    std::fs::rename(&tmp_path, path).map_err(|error| {
        armature_engine::effects::EffectError::Unsupported(format!(
            "failed to replace JSON plan file `{}`: {error}",
            path.display()
        ))
    })
}

fn plan_call_string_arg(
    request: &armature_engine::effects::EffectRequest,
    index: usize,
) -> Result<String, armature_engine::effects::EffectError> {
    let Some(value) = request
        .args
        .get("call_args")
        .and_then(serde_json::Value::as_array)
        .and_then(|args| args.get(index))
    else {
        return Err(armature_engine::effects::EffectError::Unsupported(format!(
            "effect `{}` requires string call argument {index}",
            request.effect
        )));
    };

    value.as_str().map(str::to_string).ok_or_else(|| {
        armature_engine::effects::EffectError::Unsupported(format!(
            "effect `{}` call argument {index} must be a string",
            request.effect
        ))
    })
}

fn update_task_array_status(
    plan: &mut serde_json::Value,
    work_item_id: &str,
    status: &str,
    reason: Option<&str>,
) -> Result<bool, armature_engine::effects::EffectError> {
    let Some(tasks) = plan.get_mut("tasks") else {
        return Ok(false);
    };
    let Some(tasks) = tasks.as_array_mut() else {
        return Err(armature_engine::effects::EffectError::Unsupported(
            "JSON plan field `tasks` must be an array".to_string(),
        ));
    };

    for task in tasks {
        let Some(task_object) = task.as_object_mut() else {
            continue;
        };
        if task_object.get("id").and_then(serde_json::Value::as_str) == Some(work_item_id) {
            task_object.insert(
                "status".to_string(),
                serde_json::Value::String(status.to_string()),
            );
            if let Some(reason) = reason {
                task_object.insert(
                    "blockedReason".to_string(),
                    serde_json::Value::String(reason.to_string()),
                );
            }
            return Ok(true);
        }
    }

    Err(armature_engine::effects::EffectError::Unsupported(format!(
        "JSON plan file has `tasks`, but no task with id `{work_item_id}`"
    )))
}

fn update_status_map(
    plan: &mut serde_json::Value,
    work_item_id: &str,
    status: &str,
    reason: Option<&str>,
) -> Result<(), armature_engine::effects::EffectError> {
    let Some(plan_object) = plan.as_object_mut() else {
        return Err(armature_engine::effects::EffectError::Unsupported(
            "JSON plan root must be an object".to_string(),
        ));
    };

    let statuses = plan_object
        .entry("statuses")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(statuses) = statuses.as_object_mut() else {
        return Err(armature_engine::effects::EffectError::Unsupported(
            "JSON plan field `statuses` must be an object".to_string(),
        ));
    };

    let mut item = serde_json::Map::new();
    item.insert(
        "status".to_string(),
        serde_json::Value::String(status.to_string()),
    );
    if let Some(reason) = reason {
        item.insert(
            "blockedReason".to_string(),
            serde_json::Value::String(reason.to_string()),
        );
    }
    statuses.insert(work_item_id.to_string(), serde_json::Value::Object(item));
    Ok(())
}

fn runtime_policy_allows(
    policies: &[CapabilityPolicyDocument],
    category: armature_engine::effects::EffectCategory,
    effect_name: &str,
    capability: &str,
) -> Result<(), String> {
    if policies.is_empty() {
        return Ok(());
    }

    if policies.iter().any(|policy| {
        policy
            .denied_capabilities
            .iter()
            .any(|denied| denied == capability)
    }) {
        return Err(format!(
            "effect `{effect_name}` requires denied capability `{capability}`. Fix: remove `{capability}` from denied_capabilities only if this authority is intended, otherwise remove or replace the effect."
        ));
    }

    if policies.iter().any(|policy| {
        policy
            .allowed_capabilities
            .iter()
            .any(|allowed| allowed == capability)
    }) {
        return Ok(());
    }

    let strictest_mode = policies
        .iter()
        .map(|policy| policy.mode)
        .max_by_key(policy_mode_rank)
        .unwrap_or(armature_workflow::policy::PolicyMode::Local);
    if unknown_capability_is_error(strictest_mode, category, capability) {
        Err(format!(
            "effect `{effect_name}` requires capability `{capability}` that is not allowed by supplied policy. Fix: add `{capability}` to allowed_capabilities only if this authority is intended, otherwise remove or replace the effect."
        ))
    } else {
        Ok(())
    }
}

fn successful_outcome(
    request: armature_engine::effects::EffectRequest,
    output: Option<serde_json::Value>,
    required_capabilities: Vec<String>,
) -> armature_engine::effects::EffectOutcome {
    let status = match request.category {
        armature_engine::effects::EffectCategory::AsyncInvocation
        | armature_engine::effects::EffectCategory::Message
        | armature_engine::effects::EffectCategory::HumanObligation
        | armature_engine::effects::EffectCategory::Timer => {
            armature_engine::effects::EffectOutcomeStatus::Accepted
        }
        _ => armature_engine::effects::EffectOutcomeStatus::Succeeded,
    };

    armature_engine::effects::EffectOutcome {
        effect_id: request.effect_id,
        status,
        accepted: true,
        invocation_id: matches!(
            request.category,
            armature_engine::effects::EffectCategory::AsyncInvocation
                | armature_engine::effects::EffectCategory::Message
                | armature_engine::effects::EffectCategory::HumanObligation
                | armature_engine::effects::EffectCategory::Timer
        )
        .then_some(request.idempotency_key),
        required_capabilities,
        output,
        error: None,
        completed_at: None,
    }
}

pub mod legacy {
    //! Optional compatibility adapters for old Armature concepts.
    //!
    //! Nothing in this module should be required by the workflow core.
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterEffect, AdapterManifest, AdapterModel, CapabilityPolicyDocument,
        ManifestEffectDispatcher,
    };
    use armature_engine::effects::{
        EffectCategory, EffectDispatcher, EffectOutcomeStatus, EffectRequest,
    };
    use armature_workflow::schema::Schema;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn request(effect: &str, category: EffectCategory, args: serde_json::Value) -> EffectRequest {
        EffectRequest {
            effect_id: "effect-1".to_string(),
            workflow_id: "Workflow".to_string(),
            transition_id: "transition-1".to_string(),
            idempotency_key: "Workflow:event:effect-1".to_string(),
            effect: effect.to_string(),
            category,
            target: None,
            args,
            required_capabilities: Vec::new(),
            timeout_ms: None,
        }
    }

    fn manifest() -> AdapterManifest {
        let mut effects = BTreeMap::new();
        effects.insert(
            "plan.snapshot".to_string(),
            AdapterEffect {
                category: EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.read".to_string()],
                input: Schema::Record { fields: Vec::new() },
                output: Schema::String,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Deterministic),
            },
        );
        effects.insert(
            "plan.unfinishedItems".to_string(),
            AdapterEffect {
                category: EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.read".to_string()],
                input: Schema::Json,
                output: Schema::Int,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Deterministic),
            },
        );
        effects.insert(
            "plan.nextReadyItem".to_string(),
            AdapterEffect {
                category: EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.read".to_string()],
                input: Schema::Json,
                output: Schema::Json,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Deterministic),
            },
        );
        for effect_name in [
            "plan.markReadyForQuality",
            "plan.markBlocked",
            "plan.markDone",
        ] {
            effects.insert(
                effect_name.to_string(),
                AdapterEffect {
                    category: EffectCategory::SyncValue,
                    required_capabilities: vec!["resource.plan.write".to_string()],
                    input: Schema::Json,
                    output: Schema::Json,
                    idempotent: true,
                    failure_categories: Vec::new(),
                    model: Some(AdapterModel::Opaque),
                },
            );
        }
        effects.insert(
            "start".to_string(),
            AdapterEffect {
                category: EffectCategory::AsyncInvocation,
                required_capabilities: vec!["adapter.untie.start".to_string()],
                input: Schema::Json,
                output: Schema::Json,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Opaque),
            },
        );

        AdapterManifest {
            name: "test-adapter".to_string(),
            version: "0.1.0".to_string(),
            types: BTreeMap::new(),
            effects,
            events: BTreeMap::new(),
        }
    }

    fn manifest_with_send() -> AdapterManifest {
        let mut manifest = manifest();
        manifest.effects.insert(
            "send".to_string(),
            AdapterEffect {
                category: EffectCategory::Message,
                required_capabilities: vec!["message_agents".to_string()],
                input: Schema::Json,
                output: Schema::Json,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Opaque),
            },
        );
        manifest
    }

    #[test]
    fn manifest_dispatcher_accepts_declared_effects_and_fake_outputs() {
        let mut dispatcher = ManifestEffectDispatcher::new(manifest())
            .with_fake_output("plan.snapshot", json!("plan text"));

        let outcome = dispatcher
            .dispatch(request(
                "plan.snapshot",
                EffectCategory::SyncValue,
                json!({}),
            ))
            .expect("declared effect dispatches");

        assert_eq!(outcome.status, EffectOutcomeStatus::Succeeded);
        assert_eq!(outcome.output, Some(json!("plan text")));
        assert_eq!(
            outcome.required_capabilities,
            vec!["resource.plan.read".to_string()]
        );
    }

    #[test]
    fn manifest_dispatcher_reads_and_updates_json_plan_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plan_path = dir.path().join("plan.json");
        std::fs::write(
            &plan_path,
            serde_json::to_string_pretty(&json!({
                "tasks": [
                    {"id": "W1", "status": "todo", "title": "Implement W1"},
                    {"id": "W0", "status": "done", "title": "Already done"}
                ]
            }))
            .expect("plan serializes"),
        )
        .expect("plan writes");

        let mut dispatcher =
            ManifestEffectDispatcher::new(manifest()).with_json_plan_file(&plan_path);
        let snapshot = dispatcher
            .dispatch(request(
                "plan.snapshot",
                EffectCategory::SyncValue,
                json!({}),
            ))
            .expect("snapshot succeeds");
        assert!(snapshot
            .output
            .and_then(|value| value.as_str().map(str::to_string))
            .expect("snapshot output")
            .contains("Implement W1"));

        let unfinished = dispatcher
            .dispatch(request(
                "plan.unfinishedItems",
                EffectCategory::SyncValue,
                json!({}),
            ))
            .expect("unfinished item count succeeds");
        assert_eq!(unfinished.output, Some(json!(1)));

        let next_ready = dispatcher
            .dispatch(request(
                "plan.nextReadyItem",
                EffectCategory::SyncValue,
                json!({}),
            ))
            .expect("next ready item succeeds");
        assert_eq!(next_ready.output.as_ref().expect("output")["id"], "W1");

        let ready = dispatcher
            .dispatch(request(
                "plan.markReadyForQuality",
                EffectCategory::SyncValue,
                json!({"call_args": ["W1"]}),
            ))
            .expect("status update succeeds");
        assert_eq!(ready.status, EffectOutcomeStatus::Succeeded);

        let blocked = dispatcher
            .dispatch(request(
                "plan.markBlocked",
                EffectCategory::SyncValue,
                json!({"call_args": ["W1", "needs changes"]}),
            ))
            .expect("blocked update succeeds");
        assert_eq!(blocked.status, EffectOutcomeStatus::Succeeded);

        let plan: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&plan_path).expect("updated plan reads"))
                .expect("updated plan parses");
        assert_eq!(plan["tasks"][0]["status"], "blocked");
        assert_eq!(plan["tasks"][0]["blockedReason"], "needs changes");
        assert!(!super::json_file_lock_path(&plan_path).exists());
    }

    #[test]
    fn manifest_dispatcher_writes_human_review_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let review_path = dir.path().join("reviews.json");
        let mut manifest = manifest();
        manifest
            .effects
            .extend(super::json_human_review_adapter_manifest().effects);
        let mut dispatcher =
            ManifestEffectDispatcher::new(manifest).with_human_review_file(&review_path);

        let outcome = dispatcher
            .dispatch(request(
                "askHuman",
                EffectCategory::HumanObligation,
                json!({"reason": "review needed"}),
            ))
            .expect("human review dispatch succeeds");

        assert_eq!(outcome.status, EffectOutcomeStatus::Accepted);
        let reviews: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&review_path).expect("review file reads"),
        )
        .expect("review file parses");
        assert_eq!(reviews["reviews"][0]["status"], "open");
        assert_eq!(reviews["reviews"][0]["reason"], "review needed");
        assert!(!super::json_file_lock_path(&review_path).exists());
    }

    #[test]
    fn manifest_dispatcher_writes_agent_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent_path = dir.path().join("agents.json");
        let mut manifest = manifest();
        manifest
            .effects
            .extend(super::json_agent_adapter_manifest().effects);
        let mut dispatcher = ManifestEffectDispatcher::new(manifest).with_agent_file(&agent_path);

        let mut start = request(
            "start",
            EffectCategory::AsyncInvocation,
            json!({"task": "W1", "message": "Implement W1"}),
        );
        start.target = Some("worker".to_string());
        let start_outcome = dispatcher.dispatch(start).expect("start dispatch succeeds");
        assert_eq!(start_outcome.status, EffectOutcomeStatus::Accepted);

        let mut send = request(
            "send",
            EffectCategory::Message,
            json!({"message": "please inspect"}),
        );
        send.target = Some("director".to_string());
        let send_outcome = dispatcher.dispatch(send).expect("send dispatch succeeds");
        assert_eq!(send_outcome.status, EffectOutcomeStatus::Accepted);

        let agents: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&agent_path).expect("agent file reads"))
                .expect("agent file parses");
        assert_eq!(agents["invocations"][0]["agent"], "worker");
        assert_eq!(agents["invocations"][0]["status"], "started");
        assert_eq!(agents["messages"][0]["agent"], "director");
        assert_eq!(agents["messages"][0]["message"], "please inspect");
        assert!(!super::json_file_lock_path(&agent_path).exists());
    }

    #[test]
    fn record_human_review_response_updates_review_and_deduplicates_response() {
        let dir = tempfile::tempdir().expect("tempdir");
        let review_path = dir.path().join("reviews.json");
        std::fs::write(
            &review_path,
            serde_json::to_string_pretty(&json!({
                "reviews": [
                    {"id": "review-1", "status": "open", "reason": "approve deploy"},
                    {"id": "review-2", "status": "open", "reason": "leave open"}
                ]
            }))
            .expect("review document serializes"),
        )
        .expect("review document writes");

        let payload = json!({
            "reviewId": "review-1",
            "decision": "approved",
            "response": "ship it"
        });
        super::record_human_review_response(&review_path, &payload)
            .expect("review response records");
        super::record_human_review_response(&review_path, &payload)
            .expect("duplicate review response is idempotent");

        let reviews: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&review_path).expect("review file reads"),
        )
        .expect("review file parses");
        assert_eq!(reviews["reviews"][0]["status"], "responded");
        assert_eq!(reviews["reviews"][0]["decision"], "approved");
        assert_eq!(reviews["reviews"][0]["response"], "ship it");
        assert_eq!(reviews["reviews"][1]["status"], "open");
        assert_eq!(
            reviews["responses"]
                .as_array()
                .expect("responses array")
                .len(),
            1
        );
        assert_eq!(reviews["responses"][0], payload);
        assert!(!super::json_file_lock_path(&review_path).exists());
    }

    #[test]
    fn record_agent_finished_event_marks_latest_matching_invocation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent_path = dir.path().join("agents.json");
        std::fs::write(
            &agent_path,
            serde_json::to_string_pretty(&json!({
                "invocations": [
                    {"id": "start-1", "agent": "worker", "status": "started"},
                    {"id": "start-2", "agent": "worker", "status": "started"},
                    {"id": "quality-1", "agent": "quality", "status": "started"}
                ],
                "messages": []
            }))
            .expect("agent document serializes"),
        )
        .expect("agent document writes");

        let payload = json!({
            "id": "run-99",
            "name": "worker-99",
            "status": "succeeded",
            "stdoutTail": "done",
            "stderrTail": "",
            "exitCode": 0
        });
        super::record_agent_finished_event(&agent_path, &payload)
            .expect("agent completion records");
        super::record_agent_finished_event(&agent_path, &payload)
            .expect("duplicate agent completion is idempotent");

        let agents: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&agent_path).expect("agent file reads"))
                .expect("agent file parses");
        assert_eq!(agents["invocations"][0]["status"], "started");
        assert_eq!(agents["invocations"][1]["status"], "finished");
        assert_eq!(agents["invocations"][1]["completion_id"], "run-99");
        assert_eq!(agents["invocations"][1]["completion_status"], "succeeded");
        assert_eq!(agents["invocations"][1]["exit_code"], 0);
        assert_eq!(agents["invocations"][2]["status"], "started");
        assert_eq!(
            agents["completions"]
                .as_array()
                .expect("completions array")
                .len(),
            1
        );
        assert_eq!(agents["completions"][0], payload);
        assert!(!super::json_file_lock_path(&agent_path).exists());
    }

    #[test]
    fn manifest_dispatcher_rejects_undeclared_effects() {
        let mut dispatcher = ManifestEffectDispatcher::new(manifest());

        let error = dispatcher
            .dispatch(request(
                "missing.effect",
                EffectCategory::SyncValue,
                json!({}),
            ))
            .expect_err("undeclared effect rejected");

        assert!(error
            .to_string()
            .contains("is not declared by any adapter manifest"));
    }

    #[test]
    fn manifest_dispatcher_validates_category_input_and_output() {
        let mut dispatcher = ManifestEffectDispatcher::new(manifest());
        let category_error = dispatcher
            .dispatch(request("plan.snapshot", EffectCategory::Message, json!({})))
            .expect_err("category mismatch rejected");
        assert!(category_error.to_string().contains("has category"));

        let mut dispatcher = ManifestEffectDispatcher::new(manifest());
        let input_error = dispatcher
            .dispatch(request(
                "plan.snapshot",
                EffectCategory::SyncValue,
                json!("not an object"),
            ))
            .expect_err("input mismatch rejected");
        assert!(input_error.to_string().contains("input does not match"));

        let mut dispatcher =
            ManifestEffectDispatcher::new(manifest()).with_fake_output("plan.snapshot", json!(42));
        let output_error = dispatcher
            .dispatch(request(
                "plan.snapshot",
                EffectCategory::SyncValue,
                json!({}),
            ))
            .expect_err("output mismatch rejected");
        assert!(output_error
            .to_string()
            .contains("fake output does not match"));
    }

    #[test]
    fn manifest_dispatcher_marks_async_effects_accepted() {
        let mut dispatcher = ManifestEffectDispatcher::new(manifest());

        let outcome = dispatcher
            .dispatch(request(
                "start",
                EffectCategory::AsyncInvocation,
                json!({"input": {"task": "do work"}}),
            ))
            .expect("declared async effect dispatches");

        assert_eq!(outcome.status, EffectOutcomeStatus::Accepted);
        assert_eq!(
            outcome.invocation_id,
            Some("Workflow:event:effect-1".to_string())
        );
        assert_eq!(
            outcome.required_capabilities,
            vec!["adapter.untie.start".to_string()]
        );
    }

    #[test]
    fn manifest_dispatcher_enforces_denied_policy_at_runtime() {
        let policy = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Enterprise,
            allowed_capabilities: vec!["resource.plan.read".to_string()],
            denied_capabilities: vec!["resource.plan.read".to_string()],
            ..Default::default()
        };
        let mut dispatcher = ManifestEffectDispatcher::new(manifest()).with_policies(vec![policy]);

        let error = dispatcher
            .dispatch(request(
                "plan.snapshot",
                EffectCategory::SyncValue,
                json!({}),
            ))
            .expect_err("denied capability rejected");

        assert!(error
            .to_string()
            .contains("requires denied capability `resource.plan.read`"));
        assert!(error
            .to_string()
            .contains("Fix: remove `resource.plan.read` from denied_capabilities"));
        assert_eq!(
            error.required_capabilities(),
            &["resource.plan.read".to_string()]
        );
    }

    #[test]
    fn manifest_dispatcher_allows_unknown_local_policy_at_runtime() {
        let policy = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Local,
            allowed_capabilities: Vec::new(),
            denied_capabilities: Vec::new(),
            ..Default::default()
        };
        let mut dispatcher = ManifestEffectDispatcher::new(manifest())
            .with_policies(vec![policy])
            .with_fake_output("plan.snapshot", json!("plan text"));

        let outcome = dispatcher
            .dispatch(request(
                "plan.snapshot",
                EffectCategory::SyncValue,
                json!({}),
            ))
            .expect("local unknown capability is warning-only");

        assert_eq!(outcome.status, EffectOutcomeStatus::Succeeded);
    }

    #[test]
    fn baml_http_policy_requires_enterprise_network_and_url_allowlist() {
        let policy = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Enterprise,
            allowed_capabilities: vec!["baml.coerce".to_string()],
            allow_baml_network: Some(true),
            allowed_baml_urls: vec!["http://127.0.0.1:2024".to_string()],
            ..Default::default()
        };

        assert!(crate::validate_baml_http_policy(
            std::slice::from_ref(&policy),
            "http://127.0.0.1:2024",
        )
        .is_empty());

        let diagnostics = crate::validate_baml_http_policy(&[policy], "http://127.0.0.1:2025");
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("is not allowed")));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("Fix: add the exact URL to allowed_baml_urls")));
    }

    #[test]
    fn baml_http_policy_rejects_denied_capability_and_network() {
        let policy = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Enterprise,
            denied_capabilities: vec!["baml.coerce".to_string()],
            allow_baml_network: Some(false),
            ..Default::default()
        };

        let diagnostics = crate::validate_baml_http_policy(&[policy], "http://127.0.0.1:2024");
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("denied capability `baml.coerce`")));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("Fix: remove `baml.coerce` from denied_capabilities")));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("network execution is denied")));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("Fix: set `allow_baml_network: true`")));
    }

    #[test]
    fn baml_raw_response_policy_redacts_enterprise_by_default() {
        let enterprise = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Enterprise,
            ..Default::default()
        };
        assert!(!crate::should_store_baml_raw_response(&[enterprise]));

        let opted_in = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Enterprise,
            store_baml_raw_responses: Some(true),
            ..Default::default()
        };
        assert!(crate::should_store_baml_raw_response(&[opted_in]));

        let denied = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Local,
            store_baml_raw_responses: Some(false),
            ..Default::default()
        };
        assert!(!crate::should_store_baml_raw_response(&[denied]));
    }

    #[test]
    fn manifest_validation_rejects_invalid_contracts() {
        let mut manifest = manifest();
        manifest.name = String::new();
        manifest.types.insert(
            "BadRecord".to_string(),
            Schema::Record {
                fields: vec![
                    armature_workflow::schema::Field {
                        name: "status".to_string(),
                        schema: Schema::String,
                    },
                    armature_workflow::schema::Field {
                        name: "status".to_string(),
                        schema: Schema::Int,
                    },
                ],
            },
        );
        manifest.types.insert(
            "BadEnum".to_string(),
            Schema::Enum {
                values: vec!["Ready".to_string(), "Ready".to_string()],
            },
        );
        manifest.effects.insert(
            "send".to_string(),
            AdapterEffect {
                category: EffectCategory::Message,
                required_capabilities: vec![
                    "message_agents".to_string(),
                    "message_agents".to_string(),
                    String::new(),
                ],
                input: Schema::Ref {
                    name: "Missing".to_string(),
                },
                output: Schema::Map {
                    key: Box::new(Schema::Int),
                    value: Box::new(Schema::String),
                },
                idempotent: false,
                failure_categories: vec![
                    "adapter_failure".to_string(),
                    "adapter_failure".to_string(),
                    "bad category".to_string(),
                ],
                model: Some(AdapterModel::NondeterministicOutcome {
                    values: vec![
                        "accepted".to_string(),
                        "accepted".to_string(),
                        "bad value".to_string(),
                    ],
                }),
            },
        );

        let diagnostics = crate::validate_adapter_manifests(&[manifest]);

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("name must not be empty")));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("repeats required capability")));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("references unknown type")));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("must be idempotent")));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("repeats failure category")));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("failure category `bad category`")));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("repeats nondeterministic model value")
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("nondeterministic model value `bad value`")
        }));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("type `BadRecord` repeats record field `status`")));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("type `BadEnum` repeats enum value `Ready`")));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("effect `send` output declares map key type `int`")));
    }

    #[test]
    fn manifest_validation_rejects_cyclic_type_refs() {
        let mut manifest = manifest();
        manifest.types.insert(
            "A".to_string(),
            Schema::Record {
                fields: vec![armature_workflow::schema::Field {
                    name: "b".to_string(),
                    schema: Schema::Ref {
                        name: "B".to_string(),
                    },
                }],
            },
        );
        manifest.types.insert(
            "B".to_string(),
            Schema::Ref {
                name: "A".to_string(),
            },
        );

        let diagnostics = crate::validate_adapter_manifests(&[manifest]);

        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("type `A` has a cyclic reference")));
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("type `B` has a cyclic reference")));
    }

    #[test]
    fn manifest_dispatcher_resolves_manifest_type_refs() {
        let mut manifest = manifest();
        manifest.types.insert(
            "PlanSnapshot".to_string(),
            Schema::Record {
                fields: vec![armature_workflow::schema::Field {
                    name: "text".to_string(),
                    schema: Schema::String,
                }],
            },
        );
        manifest.effects.insert(
            "plan.typedSnapshot".to_string(),
            AdapterEffect {
                category: EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.read".to_string()],
                input: Schema::Record { fields: Vec::new() },
                output: Schema::Ref {
                    name: "PlanSnapshot".to_string(),
                },
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Deterministic),
            },
        );
        let diagnostics = crate::validate_adapter_manifests(&[manifest.clone()]);
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");

        let mut dispatcher = ManifestEffectDispatcher::new(manifest)
            .with_fake_output("plan.typedSnapshot", json!({"text": "plan"}));

        let outcome = dispatcher
            .dispatch(request(
                "plan.typedSnapshot",
                EffectCategory::SyncValue,
                json!({}),
            ))
            .expect("typed ref output dispatches");

        assert_eq!(outcome.output, Some(json!({"text": "plan"})));
    }

    #[test]
    fn workflow_effect_validation_rejects_missing_manifest_effects() {
        let source = r#"
machine ManifestValidation
initial waiting

agent director = adapter("director")

event go {
  message string
}

state waiting {
  on go as evt {
    send director evt.message
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest()]);

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("effect `send` is not declared")));
    }

    #[test]
    fn workflow_policy_rejects_denied_capabilities() {
        let source = r#"
machine PolicyValidation
initial waiting

agent director = adapter("director")

event go {
  message string
}

state waiting {
  on go as evt {
    send director evt.message
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let policy = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Enterprise,
            allowed_capabilities: vec!["message_agents".to_string()],
            denied_capabilities: vec!["message_agents".to_string()],
            ..Default::default()
        };

        let diagnostics = crate::validate_workflow_policy(&ir, &[manifest_with_send()], &[policy]);

        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("denied capability `message_agents`")
            })
            .expect("denied capability diagnostic");
        assert_eq!(diagnostic.severity, armature_workflow::Severity::Error);
        assert!(diagnostic
            .message
            .contains("Fix: remove `message_agents` from denied_capabilities"));
        assert_eq!(
            diagnostic.span.as_ref().map(|span| span.start_line),
            Some(13)
        );
    }

    #[test]
    fn workflow_policy_warns_for_unknown_local_capabilities() {
        let source = r#"
machine PolicyValidation
initial waiting

capability plan = adapter("implementationPlan")

state waiting {
  entry {
    let planText = plan.snapshot()
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let policy = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Local,
            allowed_capabilities: Vec::new(),
            denied_capabilities: Vec::new(),
            ..Default::default()
        };

        let diagnostics = crate::validate_workflow_policy(&ir, &[manifest()], &[policy]);

        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("requires capability `resource.plan.read`")
            })
            .expect("unknown capability diagnostic");
        assert_eq!(diagnostic.severity, armature_workflow::Severity::Warning);
        assert!(diagnostic
            .message
            .contains("Fix: add `resource.plan.read` to allowed_capabilities"));
    }

    #[test]
    fn workflow_policy_rejects_unknown_team_write_capabilities() {
        let source = r#"
machine PolicyValidation
initial waiting

capability plan = adapter("implementationPlan")

event go {
  workItemId string
}

state waiting {
  on go as evt {
    plan.markDone(evt.workItemId)
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest();
        manifest.effects.insert(
            "plan.markDone".to_string(),
            AdapterEffect {
                category: EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.write".to_string()],
                input: Schema::Json,
                output: Schema::Json,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Opaque),
            },
        );
        let policy = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Team,
            allowed_capabilities: Vec::new(),
            denied_capabilities: Vec::new(),
            ..Default::default()
        };

        let diagnostics = crate::validate_workflow_policy(&ir, &[manifest], &[policy]);

        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic
                    .message
                    .contains("requires capability `resource.plan.write`")
            })
            .expect("unknown write capability diagnostic");
        assert_eq!(diagnostic.severity, armature_workflow::Severity::Error);
        assert!(diagnostic
            .message
            .contains("Fix: add `resource.plan.write` to allowed_capabilities"));
    }

    #[test]
    fn workflow_policy_checks_capabilities_in_invariants() {
        let source = r#"
machine PolicyValidation
initial waiting

capability plan = adapter("implementationPlan")

state waiting {
  final
}
"#;
        let mut ir = armature_workflow::parse_source(source).expect("source parses");
        ir.invariants
            .push(armature_workflow::ir::Invariant::Expression {
                name: "needsPlanRead".to_string(),
                expr: armature_workflow::expr::Expr::Call {
                    name: "plan.count".to_string(),
                    args: Vec::new(),
                },
                span: None,
            });
        let mut manifest = manifest();
        manifest.effects.insert(
            "plan.count".to_string(),
            AdapterEffect {
                category: EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.read".to_string()],
                input: Schema::Record { fields: Vec::new() },
                output: Schema::Int,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Deterministic),
            },
        );
        let policy = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Enterprise,
            allowed_capabilities: Vec::new(),
            denied_capabilities: Vec::new(),
            ..Default::default()
        };

        let diagnostics = crate::validate_workflow_policy(&ir, &[manifest], &[policy]);

        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.message.contains("invariant `needsPlanRead`")
                    && diagnostic
                        .message
                        .contains("requires capability `resource.plan.read`")
            })
            .expect("invariant capability diagnostic");
        assert_eq!(diagnostic.severity, armature_workflow::Severity::Error);
    }

    #[test]
    fn workflow_policy_uses_source_spans_for_invariant_capabilities() {
        let source = r#"
machine PolicyValidation
initial waiting

capability plan = adapter("implementationPlan")

state waiting {
  final
}

invariant needsPlanRead {
  assert plan.count() == 0
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest();
        manifest.effects.insert(
            "plan.count".to_string(),
            AdapterEffect {
                category: EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.read".to_string()],
                input: Schema::Record { fields: Vec::new() },
                output: Schema::Int,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Deterministic),
            },
        );
        let policy = CapabilityPolicyDocument {
            mode: armature_workflow::policy::PolicyMode::Enterprise,
            allowed_capabilities: Vec::new(),
            denied_capabilities: Vec::new(),
            ..Default::default()
        };

        let diagnostics = crate::validate_workflow_policy(&ir, &[manifest], &[policy]);

        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.message.contains("invariant `needsPlanRead`")
                    && diagnostic
                        .message
                        .contains("requires capability `resource.plan.read`")
            })
            .expect("invariant capability diagnostic");
        assert_eq!(
            diagnostic.span.as_ref().map(|span| span.start_line),
            Some(11)
        );
    }

    #[test]
    fn workflow_effect_validation_accepts_declared_nested_effects() {
        let source = r#"
machine ManifestValidation
initial waiting

agent director = adapter("director")
capability plan = adapter("implementationPlan")

event go {
  message string
}

state waiting {
  on go as evt {
    case evt.message {
      matches "*" -> {
        let planText = plan.snapshot()
        send director planText
        goto done
      }
    }
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest();
        manifest.effects.insert(
            "send".to_string(),
            AdapterEffect {
                category: EffectCategory::Message,
                required_capabilities: vec!["message_agents".to_string()],
                input: Schema::Json,
                output: Schema::Json,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Opaque),
            },
        );
        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn workflow_effect_validation_checks_step_request_input_schema() {
        let source = r#"
machine ManifestValidation
initial waiting

agent director = adapter("director")

event go {}

state waiting {
  on go {
    send director "hello"
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest_with_send();
        manifest.effects.insert(
            "send".to_string(),
            AdapterEffect {
                category: EffectCategory::Message,
                required_capabilities: vec!["message_agents".to_string()],
                input: Schema::Record {
                    fields: vec![armature_workflow::schema::Field {
                        name: "message".to_string(),
                        schema: Schema::String,
                    }],
                },
                output: Schema::Json,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Opaque),
            },
        );

        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("request args")));
    }

    #[test]
    fn workflow_effect_validation_accepts_step_request_input_schema_refs() {
        let source = r#"
machine ManifestValidation
initial waiting

agent director = thread("director")

event go {}

state waiting {
  on go {
    send director "hello"
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest();
        manifest.types.insert(
            "SendInput".to_string(),
            Schema::Record {
                fields: vec![
                    armature_workflow::schema::Field {
                        name: "message".to_string(),
                        schema: Schema::String,
                    },
                    armature_workflow::schema::Field {
                        name: "agent".to_string(),
                        schema: Schema::String,
                    },
                ],
            },
        );
        manifest.effects.insert(
            "send".to_string(),
            AdapterEffect {
                category: EffectCategory::Message,
                required_capabilities: vec!["message_agents".to_string()],
                input: Schema::Ref {
                    name: "SendInput".to_string(),
                },
                output: Schema::Json,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Opaque),
            },
        );

        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn workflow_effect_validation_tracks_omitted_optional_step_args() {
        let source = r#"
machine ManifestValidation
initial waiting

agent worker = adapter("worker")

event go {}

state waiting {
  on go {
    start worker
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut agent_only_manifest = manifest();
        agent_only_manifest.effects.insert(
            "start".to_string(),
            AdapterEffect {
                category: EffectCategory::AsyncInvocation,
                required_capabilities: vec!["adapter.untie.start".to_string()],
                input: Schema::Record {
                    fields: vec![armature_workflow::schema::Field {
                        name: "agent".to_string(),
                        schema: Schema::String,
                    }],
                },
                output: Schema::Json,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Opaque),
            },
        );
        assert!(crate::validate_workflow_effects(&ir, &[agent_only_manifest]).is_empty());

        let mut input_required_manifest = manifest();
        input_required_manifest.effects.insert(
            "start".to_string(),
            AdapterEffect {
                category: EffectCategory::AsyncInvocation,
                required_capabilities: vec!["adapter.untie.start".to_string()],
                input: Schema::Record {
                    fields: vec![
                        armature_workflow::schema::Field {
                            name: "agent".to_string(),
                            schema: Schema::String,
                        },
                        armature_workflow::schema::Field {
                            name: "input".to_string(),
                            schema: Schema::Json,
                        },
                    ],
                },
                output: Schema::Json,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Opaque),
            },
        );
        let diagnostics = crate::validate_workflow_effects(&ir, &[input_required_manifest]);

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("request args")));
    }

    #[test]
    fn workflow_effect_validation_checks_expression_capability_calls() {
        let source = r#"
machine ManifestValidation
initial waiting

agent director = thread("director")
capability plan = adapter("implementationPlan")

event go {
  message string
}

state waiting {
  on go as evt {
    let planText = plan.snapshot()
    send director planText
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest();
        manifest.effects.remove("plan.snapshot");
        manifest.effects.insert(
            "send".to_string(),
            AdapterEffect {
                category: EffectCategory::Message,
                required_capabilities: vec!["message_agents".to_string()],
                input: Schema::Json,
                output: Schema::Json,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Opaque),
            },
        );
        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("effect `plan.snapshot` is not declared")));
    }

    #[test]
    fn workflow_effect_validation_uses_capability_output_types_in_step_expressions() {
        let source = r#"
machine ManifestValidation
initial waiting

agent director = thread("director")
capability plan = adapter("implementationPlan")

event go {}

state waiting {
  on go {
    send director plan.count()
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest_with_send();
        manifest.effects.insert(
            "plan.count".to_string(),
            AdapterEffect {
                category: EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.read".to_string()],
                input: Schema::Record { fields: Vec::new() },
                output: Schema::Int,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Deterministic),
            },
        );

        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("message has `int` value after adapter output inference")));
    }

    #[test]
    fn workflow_effect_validation_uses_expression_types_in_request_envelopes() {
        let source = r#"
machine ManifestValidation
initial waiting

agent worker = adapter("worker")

event go {}

state waiting {
  on go {
    start worker {
      taskId 42
    }
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest();
        manifest.effects.insert(
            "start".to_string(),
            AdapterEffect {
                category: EffectCategory::AsyncInvocation,
                required_capabilities: vec!["adapter.untie.start".to_string()],
                input: Schema::Record {
                    fields: vec![
                        armature_workflow::schema::Field {
                            name: "agent".to_string(),
                            schema: Schema::String,
                        },
                        armature_workflow::schema::Field {
                            name: "input".to_string(),
                            schema: Schema::Record {
                                fields: vec![armature_workflow::schema::Field {
                                    name: "taskId".to_string(),
                                    schema: Schema::String,
                                }],
                            },
                        },
                    ],
                },
                output: Schema::Json,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Opaque),
            },
        );

        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("request args have schema")));
    }

    #[test]
    fn workflow_effect_validation_checks_adapter_output_expression_operands() {
        let source = r#"
machine ManifestValidation
initial waiting

capability plan = adapter("implementationPlan")

event go {}

state waiting {
  on go guard plan.count() == "done" {
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest();
        manifest.effects.insert(
            "plan.count".to_string(),
            AdapterEffect {
                category: EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.read".to_string()],
                input: Schema::Record { fields: Vec::new() },
                output: Schema::Int,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Deterministic),
            },
        );

        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("compares `int` and `literal` values after adapter output inference")));
    }

    #[test]
    fn workflow_effect_validation_checks_capability_value_call_inputs() {
        let source = r#"
machine ManifestValidation
initial waiting

agent director = thread("director")
capability plan = adapter("implementationPlan")

event go {
  id string
}

state waiting {
  on go as evt {
    let owner = plan.owner(42)
    send director owner
    goto done
  }
}

state done {
  final
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest_with_send();
        manifest.effects.insert(
            "plan.owner".to_string(),
            AdapterEffect {
                category: EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.read".to_string()],
                input: Schema::String,
                output: Schema::String,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Deterministic),
            },
        );

        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("capability value call `plan.owner` input has `literal` schema")));
    }

    #[test]
    fn workflow_effect_validation_checks_adapter_calls_in_invariants() {
        let source = r#"
machine ManifestValidation
initial waiting

capability plan = adapter("implementationPlan")

state waiting {
  final
}
"#;
        let mut ir = armature_workflow::parse_source(source).expect("source parses");
        ir.invariants
            .push(armature_workflow::ir::Invariant::Expression {
                name: "planCountIsDone".to_string(),
                expr: armature_workflow::expr::Expr::Eq {
                    left: Box::new(armature_workflow::expr::Expr::Call {
                        name: "plan.count".to_string(),
                        args: Vec::new(),
                    }),
                    right: Box::new(armature_workflow::expr::Expr::Literal {
                        value: json!("done"),
                    }),
                },
                span: None,
            });
        let mut manifest = manifest();
        manifest.effects.insert(
            "plan.count".to_string(),
            AdapterEffect {
                category: EffectCategory::SyncValue,
                required_capabilities: vec!["resource.plan.read".to_string()],
                input: Schema::Record { fields: Vec::new() },
                output: Schema::Int,
                idempotent: true,
                failure_categories: Vec::new(),
                model: Some(AdapterModel::Deterministic),
            },
        );

        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("invariant `planCountIsDone` compares `int` and `literal`")));
    }

    #[test]
    fn workflow_effect_validation_checks_adapter_event_schema_compatibility() {
        let source = r#"
machine EventCompatibility
initial waiting

event finished {
  name string
}

state waiting {
  on finished as evt {
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest();
        manifest.events.insert(
            "finished".to_string(),
            Schema::Record {
                fields: vec![armature_workflow::schema::Field {
                    name: "exitCode".to_string(),
                    schema: Schema::Int,
                }],
            },
        );

        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("event `finished` schema does not match")));
    }

    #[test]
    fn workflow_effect_validation_accepts_adapter_event_schema_refs() {
        let source = r#"
machine EventCompatibility
initial waiting

class Finished {
  name string
}

event finished {
  name string
}

state waiting {
  on finished as evt {
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest();
        manifest.types.insert(
            "Finished".to_string(),
            Schema::Record {
                fields: vec![armature_workflow::schema::Field {
                    name: "name".to_string(),
                    schema: Schema::String,
                }],
            },
        );
        manifest.events.insert(
            "finished".to_string(),
            Schema::Ref {
                name: "Finished".to_string(),
            },
        );

        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn workflow_effect_validation_treats_event_schema_order_as_structural() {
        let source = r#"
machine EventCompatibility
initial waiting

event finished {
  name string
  exitCode int
}

state waiting {
  on finished as evt {
    stay
  }
}
"#;
        let ir = armature_workflow::parse_source(source).expect("source parses");
        let mut manifest = manifest();
        manifest.events.insert(
            "finished".to_string(),
            Schema::Record {
                fields: vec![
                    armature_workflow::schema::Field {
                        name: "exitCode".to_string(),
                        schema: Schema::Int,
                    },
                    armature_workflow::schema::Field {
                        name: "name".to_string(),
                        schema: Schema::String,
                    },
                ],
            },
        );

        let diagnostics = crate::validate_workflow_effects(&ir, &[manifest]);

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
