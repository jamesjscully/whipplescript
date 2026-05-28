//! Loft effect provider contracts.

use std::{
    collections::BTreeMap,
    process::{Command, Stdio},
};

use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoftAction {
    Show,
    Claim,
    Renew,
    Release,
    Note,
    Transition,
    Evidence,
    ResourceIntent,
    Complete,
    Fail,
}

impl LoftAction {
    pub fn effect_kind(self) -> &'static str {
        match self {
            Self::Show => "loft.show",
            Self::Claim => "loft.claim",
            Self::Renew => "loft.renew",
            Self::Release => "loft.release",
            Self::Note => "loft.note",
            Self::Transition => "loft.transition",
            Self::Evidence => "loft.evidence",
            Self::ResourceIntent => "loft.resource_intent",
            Self::Complete => "loft.complete",
            Self::Fail => "loft.fail",
        }
    }

    fn command_name(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Claim => "claim",
            Self::Renew => "renew",
            Self::Release => "release",
            Self::Note => "note",
            Self::Transition => "set",
            Self::Evidence => "evidence",
            Self::ResourceIntent => "set",
            Self::Complete => "complete",
            Self::Fail => "fail",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoftEffectRequest {
    pub action: LoftAction,
    pub issue_id: String,
    pub lease_id: Option<String>,
    pub claim_ready: bool,
    pub issue_version: Option<String>,
    pub actor: Option<String>,
    pub lease_duration_seconds: Option<u64>,
    pub command_id: String,
    pub note: Option<String>,
    pub target_status: Option<String>,
    pub evidence_json: Option<String>,
    pub evidence_kind: Option<String>,
    pub evidence_artifact: Option<String>,
    pub evidence_data_path: Option<String>,
    pub resource_intent_json: Option<String>,
    pub release_after_failure: bool,
    pub expect_heads: Vec<String>,
    pub metadata_json: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoftEffectStatus {
    Succeeded,
    Failed,
    TimedOut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoftEffectResult {
    pub status: LoftEffectStatus,
    pub value_json: Option<String>,
    pub error_json: Option<String>,
    pub summary: String,
    pub transcript: String,
}

pub trait LoftClient {
    fn execute(&self, request: &LoftEffectRequest) -> LoftEffectResult;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandLoftClient {
    executable: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    pass_command_id: bool,
}

impl CommandLoftClient {
    pub fn new(executable: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            pass_command_id: false,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn pass_command_id(mut self, pass_command_id: bool) -> Self {
        self.pass_command_id = pass_command_id;
        self
    }

    fn command_args(&self, request: &LoftEffectRequest) -> Vec<String> {
        let mut args = self.args.clone();
        args.push(request.action.command_name().to_owned());
        match request.action {
            LoftAction::Show => {
                args.push(request.issue_id.clone());
            }
            LoftAction::Claim if request.claim_ready => {
                args.push("--ready".to_owned());
            }
            LoftAction::Renew => {
                args.push(
                    request
                        .lease_id
                        .clone()
                        .unwrap_or_else(|| request.issue_id.clone()),
                );
            }
            LoftAction::Release => {
                args.push(
                    request
                        .lease_id
                        .clone()
                        .unwrap_or_else(|| request.issue_id.clone()),
                );
            }
            LoftAction::Transition => {
                args.push(request.issue_id.clone());
                args.push("status".to_owned());
                if let Some(target_status) = &request.target_status {
                    args.push(target_status.clone());
                }
                if let Some(lease_id) = &request.lease_id {
                    args.push("--lease-id".to_owned());
                    args.push(lease_id.clone());
                }
            }
            LoftAction::Note => {
                args.push(request.issue_id.clone());
                if let Some(note) = &request.note {
                    args.push(note.clone());
                }
                if let Some(lease_id) = &request.lease_id {
                    args.push("--lease-id".to_owned());
                    args.push(lease_id.clone());
                }
            }
            LoftAction::Evidence => {
                args.push("add".to_owned());
                args.push(request.issue_id.clone());
                if let Some(lease_id) = &request.lease_id {
                    args.push("--lease-id".to_owned());
                    args.push(lease_id.clone());
                }
                if let Some(kind) = &request.evidence_kind {
                    args.push("--kind".to_owned());
                    args.push(kind.clone());
                }
                if let Some(artifact) = &request.evidence_artifact {
                    args.push("--artifact".to_owned());
                    args.push(artifact.clone());
                }
                if let Some(evidence_data_path) = request
                    .evidence_data_path
                    .as_ref()
                    .or(request.evidence_json.as_ref())
                {
                    args.push("--json-data".to_owned());
                    args.push(evidence_data_path.clone());
                }
            }
            LoftAction::ResourceIntent => {
                args.push(request.issue_id.clone());
                args.push("resource_intent".to_owned());
                args.push(
                    request
                        .resource_intent_json
                        .clone()
                        .unwrap_or_else(|| "{\"reads\":[],\"writes\":[]}".to_owned()),
                );
                if let Some(lease_id) = &request.lease_id {
                    args.push("--lease-id".to_owned());
                    args.push(lease_id.clone());
                }
            }
            LoftAction::Complete => {
                args.push(
                    request
                        .lease_id
                        .clone()
                        .unwrap_or_else(|| request.issue_id.clone()),
                );
                if let Some(reason) = &request.note {
                    args.push("--reason".to_owned());
                    args.push(reason.clone());
                }
            }
            LoftAction::Fail => {
                args.push(
                    request
                        .lease_id
                        .clone()
                        .unwrap_or_else(|| request.issue_id.clone()),
                );
                if let Some(note) = &request.note {
                    args.push("--note".to_owned());
                    args.push(note.clone());
                }
                if request.release_after_failure {
                    args.push("--release".to_owned());
                }
            }
            _ => {
                args.push(request.issue_id.clone());
            }
        }
        args.push("--json".to_owned());
        if self.pass_command_id {
            args.push("--command-id".to_owned());
            args.push(request.command_id.clone());
        }

        if let Some(issue_version) = &request.issue_version {
            args.push("--issue-version".to_owned());
            args.push(issue_version.clone());
        }
        if let Some(actor) = &request.actor {
            args.push("--actor".to_owned());
            args.push(actor.clone());
        }
        if let Some(lease_duration_seconds) = request.lease_duration_seconds {
            args.push("--ttl".to_owned());
            args.push(format!("{lease_duration_seconds}s"));
        }
        if !request.expect_heads.is_empty() {
            args.push("--expect-heads".to_owned());
            args.push(request.expect_heads.join(","));
        }
        args
    }
}

impl Default for CommandLoftClient {
    fn default() -> Self {
        Self::new("loft")
    }
}

impl LoftClient for CommandLoftClient {
    fn execute(&self, request: &LoftEffectRequest) -> LoftEffectResult {
        let args = self.command_args(request);
        let mut command = Command::new(&self.executable);
        command.args(&args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        for (key, value) in &self.env {
            command.env(key, value);
        }

        let transcript = format!("{} {}", self.executable, args.join(" "));
        let output = match command.output() {
            Ok(output) => output,
            Err(error) => {
                return LoftEffectResult {
                    status: LoftEffectStatus::Failed,
                    value_json: None,
                    error_json: Some(
                        json!({
                            "issue_id": request.issue_id,
                            "action": request.action.effect_kind(),
                            "reason": format!("could not execute Loft CLI: {error}"),
                            "recoverable": true,
                        })
                        .to_string(),
                    ),
                    summary: format!("{} failed before execution", request.action.effect_kind()),
                    transcript,
                };
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if !output.status.success() {
            return LoftEffectResult {
                status: LoftEffectStatus::Failed,
                value_json: None,
                error_json: Some(
                    json!({
                        "issue_id": request.issue_id,
                        "action": request.action.effect_kind(),
                        "exit_code": output.status.code(),
                        "stderr": stderr,
                        "stdout": json_from_str(&stdout),
                        "recoverable": true,
                    })
                    .to_string(),
                ),
                summary: format!("{} failed", request.action.effect_kind()),
                transcript: format!("{transcript}\n{stderr}"),
            };
        }

        decode_success_response(request, &stdout, transcript)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeLoftClient {
    result: LoftEffectResult,
}

impl FakeLoftClient {
    pub fn succeeds(value_json: impl Into<String>) -> Self {
        Self {
            result: LoftEffectResult {
                status: LoftEffectStatus::Succeeded,
                value_json: Some(value_json.into()),
                error_json: None,
                summary: "loft effect succeeded".to_owned(),
                transcript: "fake loft transcript\n".to_owned(),
            },
        }
    }

    pub fn fails(reason: impl Into<String>) -> Self {
        Self {
            result: LoftEffectResult {
                status: LoftEffectStatus::Failed,
                value_json: None,
                error_json: Some(
                    json!({
                        "reason": reason.into(),
                        "recoverable": true,
                    })
                    .to_string(),
                ),
                summary: "loft effect failed".to_owned(),
                transcript: "fake loft failure transcript\n".to_owned(),
            },
        }
    }
}

impl LoftClient for FakeLoftClient {
    fn execute(&self, request: &LoftEffectRequest) -> LoftEffectResult {
        let mut result = self.result.clone();
        let request_json = json!({
            "action": request.action.effect_kind(),
            "issue_id": request.issue_id,
            "lease_id": request.lease_id,
            "claim_ready": request.claim_ready,
            "issue_version": request.issue_version,
            "actor": request.actor,
            "lease_duration_seconds": request.lease_duration_seconds,
            "command_id": request.command_id,
            "note": request.note,
            "target_status": request.target_status,
            "evidence": request.evidence_json.as_deref().map(json_from_str),
            "evidence_kind": request.evidence_kind,
            "evidence_artifact": request.evidence_artifact,
            "evidence_data_path": request.evidence_data_path,
            "resource_intent": request.resource_intent_json.as_deref().map(json_from_str),
            "release_after_failure": request.release_after_failure,
            "expect_heads": request.expect_heads,
            "metadata": json_from_str(&request.metadata_json),
        });
        result.transcript.push_str(&request_json.to_string());
        result
    }
}

fn decode_success_response(
    request: &LoftEffectRequest,
    stdout: &str,
    transcript: String,
) -> LoftEffectResult {
    let body = json_from_str(stdout);
    let status = body
        .get("status")
        .and_then(Value::as_str)
        .and_then(status_from_wire)
        .unwrap_or(LoftEffectStatus::Succeeded);
    let summary = body
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or(match status {
            LoftEffectStatus::Succeeded => "loft effect succeeded",
            LoftEffectStatus::Failed => "loft effect failed",
            LoftEffectStatus::TimedOut => "loft effect timed out",
        })
        .to_owned();
    LoftEffectResult {
        status,
        value_json: body
            .get("value")
            .map(Value::to_string)
            .or_else(|| Some(body.to_string())),
        error_json: body.get("error").map(Value::to_string),
        summary,
        transcript: body
            .get("transcript")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "{transcript}\n{} {}",
                    request.action.effect_kind(),
                    request.command_id
                )
            }),
    }
}

fn status_from_wire(value: &str) -> Option<LoftEffectStatus> {
    match value {
        "succeeded" | "success" | "ok" => Some(LoftEffectStatus::Succeeded),
        "failed" | "failure" | "error" => Some(LoftEffectStatus::Failed),
        "timed_out" | "timeout" => Some(LoftEffectStatus::TimedOut),
        _ => None,
    }
}

fn json_from_str(source: &str) -> Value {
    serde_json::from_str(source).unwrap_or_else(|_| Value::String(source.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn fake_client_returns_claim_value() {
        let client = FakeLoftClient::succeeds(r#"{"claim_id":"claim-1"}"#);
        let result = client.execute(&LoftEffectRequest {
            action: LoftAction::Claim,
            issue_id: "ISSUE-1".to_owned(),
            lease_id: None,
            claim_ready: false,
            issue_version: None,
            actor: Some("whippletree".to_owned()),
            lease_duration_seconds: Some(3600),
            command_id: "cmd-1".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        });

        assert_eq!(result.status, LoftEffectStatus::Succeeded);
        assert_eq!(
            result.value_json.as_deref(),
            Some(r#"{"claim_id":"claim-1"}"#)
        );
        assert!(result.transcript.contains("ISSUE-1"));
    }

    #[test]
    fn command_client_decodes_json_stdout() {
        let client = CommandLoftClient::new("sh")
            .arg("-c")
            .arg("printf '%s' '{\"status\":\"succeeded\",\"value\":{\"claim_id\":\"claim-1\"},\"summary\":\"claimed\"}'");
        let result = client.execute(&LoftEffectRequest {
            action: LoftAction::Claim,
            issue_id: "ISSUE-1".to_owned(),
            lease_id: None,
            claim_ready: false,
            issue_version: None,
            actor: Some("whippletree".to_owned()),
            lease_duration_seconds: None,
            command_id: "cmd-1".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        });

        assert_eq!(result.status, LoftEffectStatus::Succeeded);
        assert_eq!(result.summary, "claimed");
        assert_eq!(
            result.value_json.as_deref(),
            Some(r#"{"claim_id":"claim-1"}"#)
        );
    }

    #[test]
    fn command_args_match_loft_lease_and_status_spec() {
        let claim = LoftEffectRequest {
            action: LoftAction::Claim,
            issue_id: "iss_abc".to_owned(),
            lease_id: None,
            claim_ready: true,
            issue_version: None,
            actor: Some("agent-a".to_owned()),
            lease_duration_seconds: Some(1800),
            command_id: "cmd-claim".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        };
        let show = LoftEffectRequest {
            action: LoftAction::Show,
            issue_id: "iss_abc".to_owned(),
            lease_id: None,
            claim_ready: false,
            issue_version: None,
            actor: None,
            lease_duration_seconds: None,
            command_id: "cmd-show".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        };
        let release = LoftEffectRequest {
            action: LoftAction::Release,
            issue_id: "iss_abc".to_owned(),
            lease_id: Some("lea_abc".to_owned()),
            claim_ready: false,
            issue_version: None,
            actor: None,
            lease_duration_seconds: None,
            command_id: "cmd-release".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        };
        let renew = LoftEffectRequest {
            action: LoftAction::Renew,
            issue_id: "iss_abc".to_owned(),
            lease_id: Some("lea_abc".to_owned()),
            claim_ready: false,
            issue_version: None,
            actor: None,
            lease_duration_seconds: Some(1800),
            command_id: "cmd-renew".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        };
        let transition = LoftEffectRequest {
            action: LoftAction::Transition,
            issue_id: "iss_abc".to_owned(),
            lease_id: None,
            claim_ready: false,
            issue_version: None,
            actor: None,
            lease_duration_seconds: None,
            command_id: "cmd-status".to_owned(),
            note: None,
            target_status: Some("in_progress".to_owned()),
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: vec!["evt_a".to_owned(), "evt_b".to_owned()],
            metadata_json: "{}".to_owned(),
        };
        let evidence = LoftEffectRequest {
            action: LoftAction::Evidence,
            issue_id: "iss_abc".to_owned(),
            lease_id: Some("lea_abc".to_owned()),
            claim_ready: false,
            issue_version: None,
            actor: None,
            lease_duration_seconds: None,
            command_id: "cmd-evidence".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: Some("whippletree.trace".to_owned()),
            evidence_artifact: Some("artifact:trace.json".to_owned()),
            evidence_data_path: Some("trace-summary.json".to_owned()),
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        };
        let resource_intent = LoftEffectRequest {
            action: LoftAction::ResourceIntent,
            issue_id: "iss_abc".to_owned(),
            lease_id: Some("lea_abc".to_owned()),
            claim_ready: false,
            issue_version: None,
            actor: None,
            lease_duration_seconds: None,
            command_id: "cmd-intent".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: Some(
                r#"{"reads":["src/lib.rs"],"writes":["src/lib.rs"]}"#.to_owned(),
            ),
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        };
        let complete = LoftEffectRequest {
            action: LoftAction::Complete,
            issue_id: "iss_abc".to_owned(),
            lease_id: Some("lea_abc".to_owned()),
            claim_ready: false,
            issue_version: None,
            actor: None,
            lease_duration_seconds: None,
            command_id: "cmd-complete".to_owned(),
            note: Some("done".to_owned()),
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        };
        let fail = LoftEffectRequest {
            action: LoftAction::Fail,
            issue_id: "iss_abc".to_owned(),
            lease_id: Some("lea_abc".to_owned()),
            claim_ready: false,
            issue_version: None,
            actor: None,
            lease_duration_seconds: None,
            command_id: "cmd-fail".to_owned(),
            note: Some("tests failed".to_owned()),
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: true,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        };
        let client = CommandLoftClient::new("loft");

        assert_eq!(
            client.command_args(&show),
            vec!["show", "iss_abc", "--json"]
        );
        assert_eq!(
            client.command_args(&claim),
            vec!["claim", "--ready", "--json", "--actor", "agent-a", "--ttl", "1800s",]
        );
        assert_eq!(
            client.command_args(&release),
            vec!["release", "lea_abc", "--json"]
        );
        assert_eq!(
            client.command_args(&renew),
            vec!["renew", "lea_abc", "--json", "--ttl", "1800s",]
        );
        assert_eq!(
            client.command_args(&transition),
            vec![
                "set",
                "iss_abc",
                "status",
                "in_progress",
                "--json",
                "--expect-heads",
                "evt_a,evt_b",
            ]
        );
        assert_eq!(
            client.command_args(&evidence),
            vec![
                "evidence",
                "add",
                "iss_abc",
                "--lease-id",
                "lea_abc",
                "--kind",
                "whippletree.trace",
                "--artifact",
                "artifact:trace.json",
                "--json-data",
                "trace-summary.json",
                "--json",
            ]
        );
        assert_eq!(
            client.command_args(&resource_intent),
            vec![
                "set",
                "iss_abc",
                "resource_intent",
                r#"{"reads":["src/lib.rs"],"writes":["src/lib.rs"]}"#,
                "--lease-id",
                "lea_abc",
                "--json",
            ]
        );
        assert_eq!(
            client.command_args(&complete),
            vec!["complete", "lea_abc", "--reason", "done", "--json",]
        );
        assert_eq!(
            client.command_args(&fail),
            vec![
                "fail",
                "lea_abc",
                "--note",
                "tests failed",
                "--release",
                "--json",
            ]
        );
    }

    #[test]
    fn loft_submodule_fixture_shapes_are_compatible() {
        let Some(fixture_dir) = loft_fixture_dir() else {
            return;
        };

        let manifest_path = fixture_dir.join("manifest.json");
        let manifest_text = std::fs::read_to_string(&manifest_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", manifest_path.display()));
        let manifest = json_from_str(&manifest_text);
        assert!(
            matches!(
                manifest.get("schema").and_then(Value::as_str),
                Some("loft.whippletree.fixtures.v1")
            ),
            "unexpected fixture manifest schema: {manifest}",
        );
        let fixtures = manifest
            .get("fixtures")
            .and_then(Value::as_array)
            .expect("fixture manifest has fixtures array");
        assert!(!fixtures.is_empty(), "fixture manifest is empty");

        for fixture in fixtures {
            let file_name = fixture
                .get("file")
                .and_then(Value::as_str)
                .expect("fixture entry has file");
            let expected_status = fixture
                .get("expected_status")
                .and_then(Value::as_str)
                .and_then(status_from_wire)
                .unwrap_or_else(|| panic!("{file_name} has unsupported expected_status"));
            let expected_error_code = fixture.get("expected_error_code").and_then(Value::as_str);
            let covers = fixture
                .get("covers")
                .and_then(Value::as_array)
                .expect("fixture entry has covers array");
            let path = fixture_dir.join(file_name);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            let result = decode_success_response(
                &LoftEffectRequest {
                    action: fixture_action(file_name, covers),
                    issue_id: "iss_fixture".to_owned(),
                    lease_id: Some("lea_fixture".to_owned()),
                    claim_ready: false,
                    issue_version: None,
                    actor: Some("whippletree-fixture".to_owned()),
                    lease_duration_seconds: Some(1800),
                    command_id: format!("cmd-{file_name}"),
                    note: None,
                    target_status: Some("in_progress".to_owned()),
                    evidence_json: None,
                    evidence_kind: None,
                    evidence_artifact: None,
                    evidence_data_path: None,
                    resource_intent_json: None,
                    release_after_failure: false,
                    expect_heads: Vec::new(),
                    metadata_json: "{}".to_owned(),
                },
                &text,
                format!("fixture {file_name}"),
            );
            assert_eq!(result.status, expected_status, "{file_name}");
            let payload = result
                .value_json
                .as_deref()
                .map(json_from_str)
                .expect("fixture decodes value_json");
            assert_eq!(
                payload
                    .get("status")
                    .and_then(Value::as_str)
                    .and_then(status_from_wire),
                Some(expected_status),
                "{file_name} has inconsistent status envelope: {payload}",
            );

            if expected_status == LoftEffectStatus::Failed {
                let error = required_object(&payload, "error", file_name);
                if let Some(expected_error_code) = expected_error_code {
                    assert_eq!(
                        error.get("code").and_then(Value::as_str),
                        Some(expected_error_code),
                        "{file_name} has wrong error code: {payload}",
                    );
                }
            }

            for cover in covers.iter().filter_map(Value::as_str) {
                assert_fixture_cover(file_name, cover, &payload);
            }
        }
    }

    fn fixture_action(file_name: &str, covers: &[Value]) -> LoftAction {
        let covers = covers.iter().filter_map(Value::as_str).collect::<Vec<_>>();
        if covers.contains(&"structured_evidence") {
            LoftAction::Evidence
        } else if covers.contains(&"resource_intent") {
            LoftAction::ResourceIntent
        } else if covers.contains(&"lifecycle_complete") {
            LoftAction::Complete
        } else if covers.contains(&"lifecycle_fail") {
            LoftAction::Fail
        } else if covers.contains(&"lease_renew") {
            LoftAction::Renew
        } else if covers.contains(&"lease_release") {
            LoftAction::Release
        } else if file_name.contains("transition") {
            LoftAction::Transition
        } else if file_name.contains("note") {
            LoftAction::Note
        } else if file_name.contains("claim") {
            LoftAction::Claim
        } else {
            LoftAction::Show
        }
    }

    fn assert_fixture_cover(file_name: &str, cover: &str, payload: &Value) {
        match cover {
            "json_envelope" => {
                assert_has_str(payload, "status", file_name);
            }
            "mutation_envelope" => {
                assert_has_str(payload, "tx_id", file_name);
                assert_non_empty_array(payload, "events", file_name);
                required_object(payload, "issue", file_name);
            }
            "rich_issue_shape" => assert_rich_issue_shape(file_name, payload),
            "issue_status_domain_field" => {
                let issue = required_object(payload, "issue", file_name);
                assert_has_str(issue, "issue_status", file_name);
                assert!(
                    issue.get("status").is_none(),
                    "{file_name} should use issue.issue_status for domain status: {issue}",
                );
            }
            "comment_added" => {
                assert_has_str(payload, "comment_id", file_name);
                let issue = required_object(payload, "issue", file_name);
                assert_non_empty_array(issue, "comments", file_name);
            }
            "structured_evidence" => {
                assert_has_str(payload, "evidence_id", file_name);
                let issue = required_object(payload, "issue", file_name);
                let evidence = issue
                    .get("evidence")
                    .and_then(Value::as_array)
                    .filter(|evidence| !evidence.is_empty())
                    .unwrap_or_else(|| panic!("{file_name} has no structured evidence: {issue}"));
                let first = &evidence[0];
                assert_has_str(first, "evidence_id", file_name);
                assert_has_str(first, "kind", file_name);
                assert_has_str(first, "artifact", file_name);
                required_object(first, "data", file_name);
            }
            "resource_intent" => {
                let issue = required_object(payload, "issue", file_name);
                let intent = required_object(issue, "resource_intent", file_name);
                assert_array(intent, "reads", file_name);
                assert_array(intent, "writes", file_name);
            }
            "lease_claim" => {
                assert_has_str(payload, "lease_id", file_name);
                assert_has_str(payload, "expires_at", file_name);
                required_object(payload, "issue", file_name);
            }
            "lease_renew" => {
                assert_has_str(payload, "lease_id", file_name);
                assert_has_str(payload, "expires_at", file_name);
            }
            "lease_release" => {
                assert_has_str(payload, "lease_id", file_name);
                assert_has_str(payload, "released_at", file_name);
            }
            "lifecycle_complete" => {
                let issue = required_object(payload, "issue", file_name);
                assert_eq!(
                    issue.get("issue_status").and_then(Value::as_str),
                    Some("closed"),
                    "{file_name} complete fixture should close the issue: {issue}",
                );
                assert_has_str(issue, "close_reason", file_name);
            }
            "lifecycle_fail" => {
                let issue = required_object(payload, "issue", file_name);
                assert_non_empty_array(issue, "comments", file_name);
            }
            "lease_conflict" | "lease_precondition_failure" => {
                let error = required_object(payload, "error", file_name);
                required_object(error, "details", file_name);
            }
            "retryable_error_details" => {
                let error = required_object(payload, "error", file_name);
                assert_eq!(
                    error.get("recoverable").and_then(Value::as_bool),
                    Some(true),
                    "{file_name} should mark retryable errors recoverable: {payload}",
                );
                required_object(error, "details", file_name);
            }
            "atomic_lifecycle_recovery" | "partial_failure" => {
                assert_has_str(payload, "tx_id", file_name);
                assert_non_empty_array(payload, "events", file_name);
                required_object(payload, "issue", file_name);
                let details = required_object(
                    required_object(payload, "error", file_name),
                    "details",
                    file_name,
                );
                assert_eq!(
                    details.get("durable_committed").and_then(Value::as_bool),
                    Some(true),
                    "{file_name} partial failure should expose durable commit state: {payload}",
                );
            }
            _ => {}
        }
    }

    fn assert_rich_issue_shape(file_name: &str, payload: &Value) {
        let issue = required_object(payload, "issue", file_name);
        for field in [
            "id",
            "title",
            "body",
            "issue_status",
            "type",
            "created_at",
            "updated_at",
            "state_token",
        ] {
            assert_has_str(issue, field, file_name);
        }
        for field in [
            "labels",
            "relations",
            "blocked_by",
            "comments",
            "evidence",
            "heads",
        ] {
            assert_array(issue, field, file_name);
        }
        assert!(
            issue.get("priority").and_then(Value::as_i64).is_some(),
            "{file_name} issue priority must be numeric: {issue}",
        );
        assert!(
            issue.get("conflicted").and_then(Value::as_bool).is_some(),
            "{file_name} issue conflicted must be boolean: {issue}",
        );
        required_object(issue, "field_conflicts", file_name);
        let intent = required_object(issue, "resource_intent", file_name);
        assert_array(intent, "reads", file_name);
        assert_array(intent, "writes", file_name);
    }

    fn required_object<'a>(value: &'a Value, field: &str, file_name: &str) -> &'a Value {
        value
            .get(field)
            .filter(|value| value.is_object())
            .unwrap_or_else(|| panic!("{file_name} missing object field {field}: {value}"))
    }

    fn assert_has_str(value: &Value, field: &str, file_name: &str) {
        assert!(
            value.get(field).and_then(Value::as_str).is_some(),
            "{file_name} missing string field {field}: {value}",
        );
    }

    fn assert_array(value: &Value, field: &str, file_name: &str) {
        assert!(
            value.get(field).and_then(Value::as_array).is_some(),
            "{file_name} missing array field {field}: {value}",
        );
    }

    fn assert_non_empty_array(value: &Value, field: &str, file_name: &str) {
        assert!(
            value
                .get(field)
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty()),
            "{file_name} missing non-empty array field {field}: {value}",
        );
    }

    fn loft_fixture_dir() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("WHIPPLETREE_LOFT_FIXTURE_DIR") {
            return Some(PathBuf::from(path));
        }

        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        [
            root.join("vendor/loft/fixtures/whippletree/v0.1"),
            root.join("examples/loft-fixtures/v0.1"),
        ]
        .into_iter()
        .find(|path| path.exists())
    }

    #[test]
    fn real_loft_show_smoke() {
        let Ok(issue_id) = std::env::var("WHIPPLETREE_LOFT_TEST_ISSUE") else {
            return;
        };
        let executable =
            std::env::var("WHIPPLETREE_LOFT_CLI").unwrap_or_else(|_| "loft".to_owned());
        let client = CommandLoftClient::new(executable);
        let result = client.execute(&LoftEffectRequest {
            action: LoftAction::Show,
            issue_id,
            lease_id: None,
            claim_ready: false,
            issue_version: None,
            actor: None,
            lease_duration_seconds: None,
            command_id: "cmd-whippletree-real-loft-show-smoke".to_owned(),
            note: None,
            target_status: None,
            evidence_json: None,
            evidence_kind: None,
            evidence_artifact: None,
            evidence_data_path: None,
            resource_intent_json: None,
            release_after_failure: false,
            expect_heads: Vec::new(),
            metadata_json: "{}".to_owned(),
        });

        assert_eq!(
            result.status,
            LoftEffectStatus::Succeeded,
            "real Loft show smoke failed: summary={} error={:?} transcript={}",
            result.summary,
            result.error_json,
            result.transcript
        );
        assert!(
            result.value_json.is_some(),
            "real Loft show smoke returned no issue JSON"
        );
    }
}
