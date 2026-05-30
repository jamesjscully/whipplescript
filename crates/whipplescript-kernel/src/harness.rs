//! Agent harness adapter contract and command-backed harnesses.

use std::{
    collections::BTreeMap,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

use serde_json::json;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTurnRequest {
    pub instance_id: String,
    pub effect_id: String,
    pub run_id: String,
    pub agent: String,
    pub profile: Option<String>,
    pub input_json: String,
    pub skill_names: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderRunStatus {
    Completed,
    Failed,
    TimedOut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderArtifact {
    pub kind: String,
    pub path: String,
    pub content_hash: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderFailure {
    pub phase: String,
    pub error_kind: String,
    pub message: String,
    pub recoverable: bool,
    pub retry_after: Option<String>,
    pub provider_session_id: Option<String>,
    pub provider_thread_id: Option<String>,
    pub raw_json: Option<String>,
}

impl ProviderFailure {
    pub fn new(
        phase: impl Into<String>,
        error_kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            phase: phase.into(),
            error_kind: error_kind.into(),
            message: message.into(),
            recoverable: false,
            retry_after: None,
            provider_session_id: None,
            provider_thread_id: None,
            raw_json: None,
        }
    }

    pub fn recoverable(mut self, recoverable: bool) -> Self {
        self.recoverable = recoverable;
        self
    }

    pub fn raw_json(mut self, raw_json: impl Into<String>) -> Self {
        self.raw_json = Some(raw_json.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderRunResult {
    pub status: ProviderRunStatus,
    pub summary: String,
    pub stdout: String,
    pub stderr: String,
    pub transcript: String,
    pub exit_code: Option<i64>,
    pub usage_json: String,
    pub artifacts: Vec<ProviderArtifact>,
    pub failure: Option<ProviderFailure>,
}

pub trait AgentHarness {
    fn run(&self, request: AgentTurnRequest) -> ProviderRunResult;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandLaunchPlan {
    pub provider: String,
    pub executable: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
}

impl CommandLaunchPlan {
    pub fn new(provider: impl Into<String>, executable: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            executable: executable.into(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandAgentHarness {
    plan: CommandLaunchPlan,
}

impl CommandAgentHarness {
    pub fn new(plan: CommandLaunchPlan) -> Self {
        Self { plan }
    }

    fn request_payload(&self, request: &AgentTurnRequest) -> String {
        json!({
            "provider": self.plan.provider,
            "instance_id": request.instance_id,
            "effect_id": request.effect_id,
            "run_id": request.run_id,
            "agent": request.agent,
            "profile": request.profile,
            "input": serde_json::from_str::<serde_json::Value>(&request.input_json)
                .unwrap_or_else(|_| serde_json::Value::String(request.input_json.clone())),
            "skills": request.skill_names,
        })
        .to_string()
    }
}

impl AgentHarness for CommandAgentHarness {
    fn run(&self, request: AgentTurnRequest) -> ProviderRunResult {
        let payload = self.request_payload(&request);
        let mut command = Command::new(&self.plan.executable);
        command.args(&self.plan.args);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        if let Some(cwd) = &self.plan.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &self.plan.env {
            command.env(key, value);
        }

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return command_failure_result(CommandFailure {
                    plan: &self.plan,
                    request: &request,
                    payload: &payload,
                    phase: "provider.launch.failed",
                    error_kind: "spawn_error",
                    summary: format!("failed to launch {}: {error}", self.plan.provider),
                    exit_code: None,
                    stdout: "",
                    stderr: &error.to_string(),
                    recoverable: true,
                });
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            if let Err(error) = stdin.write_all(payload.as_bytes()) {
                return command_failure_result(CommandFailure {
                    plan: &self.plan,
                    request: &request,
                    payload: &payload,
                    phase: "provider.stdin.failed",
                    error_kind: "stdin_write_error",
                    summary: format!("failed to write request to {}: {error}", self.plan.provider),
                    exit_code: None,
                    stdout: "",
                    stderr: &error.to_string(),
                    recoverable: true,
                });
            }
        }

        let output = match child.wait_with_output() {
            Ok(output) => output,
            Err(error) => {
                return command_failure_result(CommandFailure {
                    plan: &self.plan,
                    request: &request,
                    payload: &payload,
                    phase: "provider.stream.failed",
                    error_kind: "wait_error",
                    summary: format!("failed to wait for {}: {error}", self.plan.provider),
                    exit_code: None,
                    stdout: "",
                    stderr: &error.to_string(),
                    recoverable: true,
                });
            }
        };
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let exit_code = output.status.code().map(i64::from);
        let status = if output.status.success() {
            ProviderRunStatus::Completed
        } else {
            ProviderRunStatus::Failed
        };
        let summary = if output.status.success() {
            format!("{} completed", self.plan.provider)
        } else {
            format!(
                "{} failed with exit code {:?}",
                self.plan.provider, exit_code
            )
        };
        let transcript = command_transcript(&self.plan, &request, &payload, &stdout, &stderr);
        let failure = (!output.status.success()).then(|| {
            ProviderFailure::new("provider.exit.failed", "nonzero_exit", summary.clone())
                .recoverable(true)
        });

        ProviderRunResult {
            status,
            summary,
            stdout,
            stderr,
            transcript,
            exit_code,
            usage_json: "{}".to_owned(),
            artifacts: Vec::new(),
            failure,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexAgentHarness {
    inner: CommandAgentHarness,
}

impl CodexAgentHarness {
    pub fn new(plan: CommandLaunchPlan) -> Self {
        Self {
            inner: CommandAgentHarness::new(plan),
        }
    }
}

impl AgentHarness for CodexAgentHarness {
    fn run(&self, request: AgentTurnRequest) -> ProviderRunResult {
        self.inner.run(request)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeCodeAgentHarness {
    inner: CommandAgentHarness,
}

impl ClaudeCodeAgentHarness {
    pub fn new(plan: CommandLaunchPlan) -> Self {
        Self {
            inner: CommandAgentHarness::new(plan),
        }
    }
}

impl AgentHarness for ClaudeCodeAgentHarness {
    fn run(&self, request: AgentTurnRequest) -> ProviderRunResult {
        self.inner.run(request)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PiStyleAgentHarness {
    inner: CommandAgentHarness,
}

impl PiStyleAgentHarness {
    pub fn new(plan: CommandLaunchPlan) -> Self {
        Self {
            inner: CommandAgentHarness::new(plan),
        }
    }
}

impl AgentHarness for PiStyleAgentHarness {
    fn run(&self, request: AgentTurnRequest) -> ProviderRunResult {
        self.inner.run(request)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MockAgentHarness {
    result: ProviderRunResult,
}

impl MockAgentHarness {
    pub fn completed(summary: impl Into<String>) -> Self {
        Self {
            result: ProviderRunResult {
                status: ProviderRunStatus::Completed,
                summary: summary.into(),
                stdout: "mock stdout\n".to_owned(),
                stderr: String::new(),
                transcript: "mock transcript\n".to_owned(),
                exit_code: Some(0),
                usage_json: r#"{"input_tokens":1,"output_tokens":1}"#.to_owned(),
                artifacts: vec![ProviderArtifact {
                    kind: "transcript".to_owned(),
                    path: "artifacts/mock-transcript.txt".to_owned(),
                    content_hash: Some("mock-transcript-hash".to_owned()),
                    mime_type: Some("text/plain".to_owned()),
                }],
                failure: None,
            },
        }
    }

    pub fn failed(summary: impl Into<String>) -> Self {
        let summary = summary.into();
        Self {
            result: ProviderRunResult {
                status: ProviderRunStatus::Failed,
                summary: summary.clone(),
                stdout: String::new(),
                stderr: "mock failure\n".to_owned(),
                transcript: "mock failed transcript\n".to_owned(),
                exit_code: Some(1),
                usage_json: r#"{"input_tokens":1,"output_tokens":0}"#.to_owned(),
                artifacts: Vec::new(),
                failure: Some(ProviderFailure::new(
                    "provider.fixture.failed",
                    "fixture_failure",
                    summary,
                )),
            },
        }
    }
}

impl AgentHarness for MockAgentHarness {
    fn run(&self, request: AgentTurnRequest) -> ProviderRunResult {
        let mut result = self.result.clone();
        result.transcript.push_str(&format!(
            "agent={} effect={} run={} skills={}\n",
            request.agent,
            request.effect_id,
            request.run_id,
            request.skill_names.join(",")
        ));
        result
    }
}

struct CommandFailure<'a> {
    plan: &'a CommandLaunchPlan,
    request: &'a AgentTurnRequest,
    payload: &'a str,
    phase: &'a str,
    error_kind: &'a str,
    summary: String,
    exit_code: Option<i64>,
    stdout: &'a str,
    stderr: &'a str,
    recoverable: bool,
}

fn command_failure_result(failure: CommandFailure<'_>) -> ProviderRunResult {
    ProviderRunResult {
        status: ProviderRunStatus::Failed,
        summary: failure.summary.clone(),
        stdout: failure.stdout.to_owned(),
        stderr: failure.stderr.to_owned(),
        transcript: command_transcript(
            failure.plan,
            failure.request,
            failure.payload,
            failure.stdout,
            failure.stderr,
        ),
        exit_code: failure.exit_code,
        usage_json: "{}".to_owned(),
        artifacts: Vec::new(),
        failure: Some(
            ProviderFailure::new(failure.phase, failure.error_kind, failure.summary)
                .recoverable(failure.recoverable),
        ),
    }
}

fn command_transcript(
    plan: &CommandLaunchPlan,
    request: &AgentTurnRequest,
    payload: &str,
    stdout: &str,
    stderr: &str,
) -> String {
    format!(
        "provider={}\ncommand={} {}\ninstance={}\neffect={}\nrun={}\nrequest={}\nstdout={}\nstderr={}\n",
        plan.provider,
        plan.executable,
        plan.args.join(" "),
        request.instance_id,
        request.effect_id,
        request.run_id,
        payload,
        stdout,
        stderr,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_harness_sends_request_to_stdin_and_captures_output() {
        let harness = CommandAgentHarness::new(
            CommandLaunchPlan::new("fixture", "sh")
                .arg("-c")
                .arg("cat; echo err >&2"),
        );

        let result = harness.run(AgentTurnRequest {
            instance_id: "instance-a".to_owned(),
            effect_id: "tell".to_owned(),
            run_id: "run-tell".to_owned(),
            agent: "worker".to_owned(),
            profile: Some("repo-writer".to_owned()),
            input_json: r#"{"prompt":"go"}"#.to_owned(),
            skill_names: vec!["loft-user".to_owned()],
        });

        assert_eq!(result.status, ProviderRunStatus::Completed);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("\"effect_id\":\"tell\""));
        assert!(result.stderr.contains("err"));
        assert!(result.transcript.contains("provider=fixture"));
        assert!(result.failure.is_none());
    }

    #[test]
    fn command_harness_captures_launch_failure_as_structured_failure() {
        let harness = CommandAgentHarness::new(CommandLaunchPlan::new(
            "fixture",
            "definitely-not-a-command",
        ));

        let result = harness.run(AgentTurnRequest {
            instance_id: "instance-a".to_owned(),
            effect_id: "tell".to_owned(),
            run_id: "run-tell".to_owned(),
            agent: "worker".to_owned(),
            profile: Some("repo-writer".to_owned()),
            input_json: r#"{"prompt":"go"}"#.to_owned(),
            skill_names: Vec::new(),
        });

        let failure = result.failure.expect("failure is structured");
        assert_eq!(result.status, ProviderRunStatus::Failed);
        assert_eq!(failure.phase, "provider.launch.failed");
        assert_eq!(failure.error_kind, "spawn_error");
        assert!(failure.recoverable);
        assert!(result.transcript.contains("definitely-not-a-command"));
    }

    #[test]
    fn command_harness_captures_nonzero_exit_as_structured_failure() {
        let harness = CommandAgentHarness::new(
            CommandLaunchPlan::new("fixture", "sh")
                .arg("-c")
                .arg("cat >/dev/null; echo nope >&2; exit 42"),
        );

        let result = harness.run(AgentTurnRequest {
            instance_id: "instance-a".to_owned(),
            effect_id: "tell".to_owned(),
            run_id: "run-tell".to_owned(),
            agent: "worker".to_owned(),
            profile: Some("repo-writer".to_owned()),
            input_json: r#"{"prompt":"go"}"#.to_owned(),
            skill_names: Vec::new(),
        });

        let failure = result.failure.expect("failure is structured");
        assert_eq!(result.status, ProviderRunStatus::Failed);
        assert_eq!(result.exit_code, Some(42));
        assert_eq!(failure.phase, "provider.exit.failed");
        assert_eq!(failure.error_kind, "nonzero_exit");
        assert!(result.stderr.contains("nope"));
    }

    #[test]
    fn real_provider_adapters_delegate_to_command_plan() {
        let request = AgentTurnRequest {
            instance_id: "instance-a".to_owned(),
            effect_id: "tell".to_owned(),
            run_id: "run-tell".to_owned(),
            agent: "worker".to_owned(),
            profile: Some("repo-writer".to_owned()),
            input_json: "{}".to_owned(),
            skill_names: Vec::new(),
        };

        let codex = CodexAgentHarness::new(
            CommandLaunchPlan::new("codex", "sh")
                .arg("-c")
                .arg("cat >/dev/null"),
        );
        let claude = ClaudeCodeAgentHarness::new(
            CommandLaunchPlan::new("claude-code", "sh")
                .arg("-c")
                .arg("cat >/dev/null"),
        );
        let pi = PiStyleAgentHarness::new(
            CommandLaunchPlan::new("pi", "sh")
                .arg("-c")
                .arg("cat >/dev/null"),
        );

        assert_eq!(
            codex.run(request.clone()).status,
            ProviderRunStatus::Completed
        );
        assert_eq!(
            claude.run(request.clone()).status,
            ProviderRunStatus::Completed
        );
        assert_eq!(pi.run(request).status, ProviderRunStatus::Completed);
    }
}
