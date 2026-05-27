use serde_json::Value;
use std::process::Command;

fn armature() -> Command {
    Command::new(env!("CARGO_BIN_EXE_armature"))
}

fn formal_checks_available() -> bool {
    if std::env::var_os("ARMATURE_RUN_FORMAL_TESTS").is_none() {
        return false;
    }
    Command::new("sh")
        .arg("-c")
        .arg(
            "(command -v tlc >/dev/null 2>&1 && command -v maude >/dev/null 2>&1) || command -v nix >/dev/null 2>&1",
        )
        .status()
        .expect("tool probe runs")
        .success()
}

fn loopback_tests_enabled() -> bool {
    std::env::var_os("ARMATURE_RUN_LOOPBACK_TESTS").is_some()
}

fn workflow_source() -> &'static str {
    r#"
machine CliWorkflow
initial waiting

agent director = thread("director")

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
"#
}

fn write_workflow(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let file = dir.path().join("workflow.armature");
    std::fs::write(&file, workflow_source()).expect("workflow writes");
    file
}

fn adapter_workflow_source() -> &'static str {
    r#"
machine CliWorkflow
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
"#
}

fn write_adapter_workflow(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let file = dir.path().join("workflow.armature");
    std::fs::write(&file, adapter_workflow_source()).expect("workflow writes");
    file
}

fn run_json(command: &mut Command) -> Value {
    let output = command.output().expect("command runs");
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is json")
}

fn baml_coerce_workflow_source() -> &'static str {
    r#"
machine BamlHttpWorkflow
initial waiting

data {
  result string = ""
}

event go {
  message string
}

coerce classify(message string) -> string {
  model "gpt-5-codex"
  prompt """
Classify this message.

{{ message }}
  """
}

state waiting {
  on go as evt {
    let classification = coerce classify(evt.message)
    assign data.result = classification
    goto done
  }
}

state done {
  final
}
"#
}

fn write_baml_coerce_workflow(dir: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    let file = dir.path().join(name);
    std::fs::write(&file, baml_coerce_workflow_source()).expect("workflow writes");
    file
}

fn write_fake_baml_cli(dir: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("fake bin dir writes");
    let bin = bin_dir.join("baml-cli");
    let log = dir.path().join("fake-baml-cli.log");
    std::fs::write(
        &bin,
        r#"#!/bin/sh
python3 - "$@" <<'PY'
import http.server
import json
import os
import signal
import socketserver
import sys

log_path = os.environ["ARMATURE_FAKE_BAML_LOG"]
args = sys.argv[1:]

def log(record):
    with open(log_path, "a", encoding="utf-8") as handle:
        handle.write(json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n")

if not args or args[0] != "serve":
    log({"error": "unexpected command", "args": args})
    sys.exit(2)

from_dir = None
port = None
index = 1
while index < len(args):
    arg = args[index]
    if arg == "--from" and index + 1 < len(args):
        from_dir = args[index + 1]
        index += 2
    elif arg.startswith("--from="):
        from_dir = arg.split("=", 1)[1]
        index += 1
    elif arg == "--port" and index + 1 < len(args):
        port = int(args[index + 1])
        index += 2
    elif arg.startswith("--port="):
        port = int(arg.split("=", 1)[1])
        index += 1
    else:
        index += 1

if from_dir is None:
    log({"error": "missing --from", "args": args})
    sys.exit(2)
if port is None:
    port = 2024

baml_path = os.path.join(from_dir, "workflow.baml")
with open(baml_path, "r", encoding="utf-8") as handle:
    baml_source = handle.read()

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"ok":true}')

    def do_POST(self):
        size = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(size).decode("utf-8")
        log({"request_path": self.path, "request_body": body})
        response = b'"managed"'
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(response)))
        self.end_headers()
        self.wfile.write(response)

    def log_message(self, format, *args):
        return

socketserver.TCPServer.allow_reuse_address = True
server = socketserver.TCPServer(("127.0.0.1", port), Handler)
actual_port = server.server_address[1]
log({
    "args": args,
    "from": from_dir,
    "port": actual_port,
    "baml_contains_classify": "function Classify" in baml_source,
})
print(f"http://127.0.0.1:{actual_port}", flush=True)

def stop(_signum, _frame):
    server.server_close()
    sys.exit(0)

signal.signal(signal.SIGTERM, stop)
server.serve_forever()
PY
"#,
    )
    .expect("fake baml-cli writes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&bin)
            .expect("fake baml-cli metadata reads")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&bin, permissions).expect("fake baml-cli is executable");
    }
    (bin_dir, log)
}

fn command_with_fake_baml_cli(
    command: &mut Command,
    bin_dir: &std::path::Path,
    log: &std::path::Path,
) {
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(bin_dir.to_path_buf()).chain(std::env::split_paths(&existing_path));
    let path = std::env::join_paths(paths).expect("fake PATH builds");
    command.env("PATH", path).env("ARMATURE_FAKE_BAML_LOG", log);
}

fn write_fake_generated_stdio_runner(
    dir: &tempfile::TempDir,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let bin = dir.path().join("fake-generated-baml-runner");
    let log = dir.path().join("fake-generated-baml-runner.log");
    std::fs::write(
        &bin,
        r#"#!/usr/bin/env python3
import json
import os
import sys

log_path = os.environ["ARMATURE_FAKE_BAML_GENERATED_STDIO_LOG"]
stdin = sys.stdin.read()
payload = None
for candidate in [line for line in stdin.splitlines() if line.strip()] or [stdin]:
    if candidate.strip():
        payload = json.loads(candidate)
        break
if payload is None and len(sys.argv) > 1:
    payload = json.loads(sys.argv[1])
if payload is None:
    payload = {}

with open(log_path, "a", encoding="utf-8") as handle:
    handle.write(json.dumps({
        "argv": sys.argv[1:],
        "codex_oauth_token_present": bool(os.environ.get("ARMATURE_CODEX_OAUTH_ACCESS_TOKEN")),
        "codex_oauth_base_url": os.environ.get("ARMATURE_CODEX_OAUTH_BASE_URL"),
        "request": payload,
    }, sort_keys=True, separators=(",", ":")) + "\n")

request_id = payload.get("id") or payload.get("coerce_call_id") or "coerce_fake"
function = payload.get("function") or payload.get("function_name")
args = payload.get("args") or {}
if function != "classify":
    print(json.dumps({
        "id": request_id,
        "ok": False,
        "error": f"unexpected function {function!r}",
    }, separators=(",", ":")), flush=True)
    sys.exit(0)

print(json.dumps({
    "id": request_id,
    "coerce_call_id": request_id,
    "ok": True,
    "status": "succeeded",
    "http_status": None,
    "value": "generated-stdio",
    "parsed_output": "generated-stdio",
    "raw": {
        "fakeGeneratedRunner": True,
        "message": args.get("message"),
    },
    "raw_response": {
        "fakeGeneratedRunner": True,
        "message": args.get("message"),
    },
    "error": None,
}, separators=(",", ":")), flush=True)
"#,
    )
    .expect("fake generated stdio runner writes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&bin)
            .expect("fake generated stdio runner metadata reads")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&bin, permissions)
            .expect("fake generated stdio runner is executable");
    }
    (bin, log)
}

fn command_with_fake_generated_stdio_runner(
    command: &mut Command,
    runner: &std::path::Path,
    log: &std::path::Path,
) {
    command
        .env("ARMATURE_BAML_GENERATED_STDIO_RUNNER", runner)
        .env("ARMATURE_FAKE_BAML_GENERATED_STDIO_LOG", log);
}

fn write_fake_baml_generate_cli(
    dir: &tempfile::TempDir,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let bin_dir = dir.path().join("generate-bin");
    std::fs::create_dir_all(&bin_dir).expect("fake generate bin dir writes");
    let bin = bin_dir.join("baml-cli");
    let log = dir.path().join("fake-baml-generate.log");
    std::fs::write(
        &bin,
        r#"#!/usr/bin/env python3
import json
import os
import sys

log_path = os.environ["ARMATURE_FAKE_BAML_GENERATE_LOG"]
args = sys.argv[1:]
from_dir = None
for index, arg in enumerate(args):
    if arg == "--from" and index + 1 < len(args):
        from_dir = args[index + 1]
    elif arg.startswith("--from="):
        from_dir = arg.split("=", 1)[1]
if not args or args[0] != "generate" or from_dir is None:
    with open(log_path, "a", encoding="utf-8") as handle:
        handle.write(json.dumps({"error":"unexpected args","args":args}, sort_keys=True) + "\n")
    sys.exit(2)

with open(os.path.join(from_dir, "workflow.baml"), "r", encoding="utf-8") as handle:
    workflow_source = handle.read()
with open(os.path.join(from_dir, "generators.baml"), "r", encoding="utf-8") as handle:
    generator_source = handle.read()

runner_dir = os.path.abspath(os.path.join(from_dir, "..", "baml_runner"))
client_dir = os.path.join(runner_dir, "baml_client")
os.makedirs(client_dir, exist_ok=True)
with open(os.path.join(client_dir, "index.ts"), "w", encoding="utf-8") as handle:
    handle.write("""
export const b = {
  Classify: async function(message: string) {
    return "generated-client:" + message;
  }
};
""")

with open(log_path, "a", encoding="utf-8") as handle:
    handle.write(json.dumps({
        "args": args,
        "from": from_dir,
        "workflow_contains_classify": "function Classify" in workflow_source,
        "generator_contains_typescript": "output_type \"typescript\"" in generator_source,
        "client": os.path.join(client_dir, "index.ts"),
    }, sort_keys=True, separators=(",", ":")) + "\n")
"#,
    )
    .expect("fake baml generate writes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&bin)
            .expect("fake baml generate metadata reads")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&bin, permissions).expect("fake baml generate is executable");
    }
    (bin_dir, log)
}

fn command_with_fake_baml_generate(
    command: &mut Command,
    bin_dir: &std::path::Path,
    log: &std::path::Path,
) {
    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(bin_dir.to_path_buf()).chain(std::env::split_paths(&existing_path));
    let path = std::env::join_paths(paths).expect("fake generate PATH builds");
    let tsc = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .join("node_modules/.bin/tsc");
    command
        .env("PATH", path)
        .env("ARMATURE_TSC", tsc)
        .env("ARMATURE_FAKE_BAML_GENERATE_LOG", log);
}

#[test]
fn validate_accepts_workflow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);

    let output = run_json(armature().arg("validate").arg(file).arg("--json"));

    assert_eq!(output["ok"], true);
    assert_eq!(output["diagnostics"], Value::Array(Vec::new()));
}

#[test]
fn init_scaffolds_valid_local_project_and_refuses_overwrite() {
    let dir = tempfile::tempdir().expect("tempdir");

    let output = run_json(
        armature()
            .arg("init")
            .arg(dir.path())
            .arg("--name")
            .arg("DemoWorkflow")
            .arg("--json"),
    );

    let workflow = dir.path().join("workflow.armature");
    let policy = dir.path().join(".armature/policy.json");
    let harness_policy = dir.path().join(".armature/harness-policy.json");
    let state_dir = dir.path().join(".armature/state");
    let workflow_store_dir = dir.path().join(".armature/workflows");
    assert_eq!(output["workflow_name"], "DemoWorkflow");
    assert_eq!(
        output["workflow"].as_str(),
        Some(workflow.to_string_lossy().as_ref())
    );
    assert_eq!(
        output["policy"].as_str(),
        Some(policy.to_string_lossy().as_ref())
    );
    assert_eq!(
        output["harness_policy"].as_str(),
        Some(harness_policy.to_string_lossy().as_ref())
    );
    assert_eq!(
        output["state_dir"].as_str(),
        Some(state_dir.to_string_lossy().as_ref())
    );
    assert_eq!(
        output["workflow_store_dir"].as_str(),
        Some(workflow_store_dir.to_string_lossy().as_ref())
    );
    assert!(state_dir.is_dir());
    assert!(workflow_store_dir.is_dir());
    let workflow_source = std::fs::read_to_string(&workflow).expect("workflow reads");
    assert!(workflow_source.contains("machine DemoWorkflow"));
    let harness_policy_source =
        std::fs::read_to_string(&harness_policy).expect("harness policy reads");
    assert!(harness_policy_source.contains(r#""mode": "separated""#));
    assert!(harness_policy_source.contains(r#""repo-writer""#));

    let validation = run_json(armature().arg("validate").arg(&workflow).arg("--json"));
    assert_eq!(validation["ok"], true);

    let profile_validation = run_json(
        armature()
            .arg("validate-profile-policy")
            .arg(&harness_policy)
            .arg("--workflow")
            .arg(&workflow)
            .arg("--json"),
    );
    assert_eq!(profile_validation["ok"], true);

    let policy_validation = run_json(armature().arg("validate-policy").arg(&policy).arg("--json"));
    assert_eq!(policy_validation["ok"], true);

    let duplicate = armature()
        .arg("init")
        .arg(dir.path())
        .output()
        .expect("command runs");
    assert!(!duplicate.status.success());
    let stderr = String::from_utf8_lossy(&duplicate.stderr);
    assert!(stderr.contains("refusing to overwrite existing"));

    let forced = armature()
        .arg("init")
        .arg(dir.path())
        .arg("--force")
        .output()
        .expect("command runs");
    assert!(forced.status.success());

    let invalid = armature()
        .arg("init")
        .arg(dir.path())
        .arg("--name")
        .arg("bad name")
        .arg("--force")
        .output()
        .expect("command runs");
    assert!(!invalid.status.success());
    let stderr = String::from_utf8_lossy(&invalid.stderr);
    assert!(stderr.contains("invalid workflow name `bad name`"));
}

#[test]
fn validation_errors_are_reported_outside_validate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("bad.armature");
    std::fs::write(
        &file,
        r#"
machine BadWorkflow
initial missing

state waiting {
  final
}
"#,
    )
    .expect("workflow writes");

    let output = armature()
        .arg("emit-model")
        .arg(&file)
        .arg("--target")
        .arg("tla")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error: workflow validation failed"));
    assert!(stderr.contains("initial state `missing` is not declared"));
}

#[test]
fn validate_text_output_includes_diagnostic_locations() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("bad-syntax.armature");
    std::fs::write(
        &file,
        r#"machine
initial waiting
"#,
    )
    .expect("workflow writes");

    let output = armature()
        .arg("validate")
        .arg(&file)
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&format!(
        "{}:2:1: error: expected machine name",
        file.display()
    )));
}

#[test]
fn validate_text_output_includes_validation_locations() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("bad-validation.armature");
    std::fs::write(
        &file,
        r#"
machine BadValidation
initial done

agent worker = codingAgent() {
  maxActive 0
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");

    let output = armature()
        .arg("validate")
        .arg(&file)
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&format!(
        "{}:5:1: error: agent `worker` maxActive must be greater than 0",
        file.display()
    )));
}

#[test]
fn events_help_uses_durable_status_spellings() {
    let output = armature()
        .arg("events")
        .arg("--help")
        .output()
        .expect("command runs");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("dead_lettered"));
}

#[test]
fn core_command_help_describes_shared_options() {
    for command in ["emit", "run", "status"] {
        let output = armature()
            .arg(command)
            .arg("--help")
            .output()
            .expect("command runs");

        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("SQLite workflow store path"), "{command}");
        assert!(stdout.contains("Adapter manifest JSON file"), "{command}");
        assert!(stdout.contains("Capability policy JSON file"), "{command}");
    }

    let run_help = armature()
        .arg("run")
        .arg("--help")
        .output()
        .expect("command runs");
    let stdout = String::from_utf8_lossy(&run_help.stdout);
    assert!(stdout.contains("Workflow event type to enqueue before processing"));
    assert!(stdout.contains("JSON payload for --event"));
}

#[test]
fn emit_run_status_events_and_log_share_the_same_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let store = dir.path().join("workflow.sqlite");
    let agent_file = dir.path().join("agents.json");

    let emitted = run_json(
        armature()
            .arg("emit")
            .arg(&file)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(emitted["status"]["pending_events"], 1);
    assert_eq!(emitted["event"]["event_type"], "go");

    let events = run_json(
        armature()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(events["events"][0]["status"], "queued");

    let queued_events = run_json(
        armature()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--status")
            .arg("queued")
            .arg("--json"),
    );
    assert_eq!(queued_events["events"][0]["event_type"], "go");

    let queued_events_text = armature()
        .arg("events")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .arg("--status")
        .arg("queued")
        .output()
        .expect("command runs");
    assert!(queued_events_text.status.success());
    let stdout = String::from_utf8_lossy(&queued_events_text.stdout);
    assert!(stdout.contains(" go queued"));

    let dead_lettered_events = run_json(
        armature()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--status")
            .arg("dead_lettered")
            .arg("--json"),
    );
    assert_eq!(dead_lettered_events["events"], Value::Array(Vec::new()));

    let dead_lettered_alias_events = run_json(
        armature()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--status")
            .arg("dead-lettered")
            .arg("--json"),
    );
    assert_eq!(
        dead_lettered_alias_events["events"],
        Value::Array(Vec::new())
    );

    let processed_events_before_run = run_json(
        armature()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--status")
            .arg("processed")
            .arg("--json"),
    );
    assert_eq!(
        processed_events_before_run["events"],
        Value::Array(Vec::new())
    );

    let run = run_json(
        armature()
            .arg("run")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(run["status"]["current_state"], "done");
    assert_eq!(run["status"]["pending_events"], 0);
    assert_eq!(run["status"]["recent_effects"][0]["effect"], "send");
    assert_eq!(
        run["status"]["recent_effects"][0]["args"]["message"],
        "hello"
    );

    let processed_events = run_json(
        armature()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--status")
            .arg("processed")
            .arg("--json"),
    );
    assert_eq!(processed_events["events"][0]["event_type"], "go");
    assert_eq!(processed_events["events"][0]["status"], "processed");

    let status = run_json(
        armature()
            .arg("status")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--fixture-agent-file")
            .arg(&agent_file)
            .arg("--json"),
    );
    assert_eq!(status["current_state"], "done");
    assert_eq!(status["recent_effects"][0]["effect"], "send");
    assert_eq!(status["recent_effects"][0]["status"], "dispatched");
    assert!(status["recent_effects"][0]["idempotency_key"]
        .as_str()
        .is_some_and(|key| key.contains("CliWorkflow:")));
    assert_eq!(status["recent_effects"][0]["args"]["message"], "hello");
    assert_eq!(status["data_summary"], serde_json::json!({}));
    assert_eq!(status["policy_blockers"], serde_json::json!([]));

    let status_text = armature()
        .arg("status")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .arg("--fixture-agent-file")
        .arg(&agent_file)
        .output()
        .expect("command runs");
    assert!(status_text.status.success());
    let stdout = String::from_utf8_lossy(&status_text.stdout);
    assert!(stdout.contains("workflow: CliWorkflow"));
    assert!(stdout.contains("state: done"));
    assert!(stdout.contains("waiting: idle; no queued events or active invocations"));
    assert!(stdout.contains("data summary: none"));
    assert!(stdout.contains("latest effects:"));
    assert!(stdout.contains("send director Dispatched"));
    assert!(stdout.contains("policy blockers: none"));

    let overview = run_json(
        armature()
            .arg("overview")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--fixture-agent-file")
            .arg(&agent_file)
            .arg("--json"),
    );
    assert_eq!(overview["validation"]["ok"], true);
    assert_eq!(overview["status"]["current_state"], "done");
    assert_eq!(overview["status"]["pending_events"], 0);
    assert_eq!(overview["status"]["recent_effects"][0]["effect"], "send");
    assert_eq!(overview["status"]["data_summary"], serde_json::json!({}));
    assert_eq!(overview["status"]["policy_blockers"], serde_json::json!([]));

    let overview_text = armature()
        .arg("overview")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .arg("--fixture-agent-file")
        .arg(&agent_file)
        .output()
        .expect("command runs");
    assert!(overview_text.status.success());
    let stdout = String::from_utf8_lossy(&overview_text.stdout);
    assert!(stdout.contains("validation: ok"));
    assert!(stdout.contains("workflow: CliWorkflow"));
    assert!(stdout.contains("state: done"));
    assert!(stdout.contains("waiting: idle; no queued events or active invocations"));
    assert!(stdout.contains("data summary: none"));
    assert!(stdout.contains("latest effects:"));
    assert!(stdout.contains("policy blockers: none"));

    let log = run_json(
        armature()
            .arg("log")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(log["records"][0]["type"], "effect");
    assert_eq!(log["records"][0]["status"], "dispatched");
}

#[test]
fn retry_event_requeues_only_failed_and_dead_lettered_events() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let store = dir.path().join("workflow.sqlite");
    let missing_store = dir.path().join("missing.sqlite");

    let missing_retry = armature()
        .arg("retry-event")
        .arg(&file)
        .arg("--store")
        .arg(&missing_store)
        .arg("--event-id")
        .arg("evt_missing")
        .output()
        .expect("command runs");
    assert!(!missing_retry.status.success());
    assert!(
        !missing_store.exists(),
        "retry-event must not create an empty store when the target event is absent"
    );
    let stderr = String::from_utf8_lossy(&missing_retry.stderr);
    assert!(stderr.contains("workflow event not found"));

    let mut last_event_id = String::new();
    for retryable_status in ["failed", "dead_lettered"] {
        let emitted = run_json(
            armature()
                .arg("emit")
                .arg(&file)
                .arg("--event")
                .arg("go")
                .arg("--payload")
                .arg(r#"{"message":"retry me"}"#)
                .arg("--store")
                .arg(&store)
                .arg("--json"),
        );
        let event_id = emitted["event"]["event_id"]
            .as_str()
            .expect("event id")
            .to_string();
        let mut event_json = emitted["event"].clone();
        event_json["status"] = serde_json::json!(retryable_status);
        event_json["last_error"] = serde_json::json!("simulated failure");

        let connection = rusqlite::Connection::open(&store).expect("store opens");
        connection
            .execute(
                "UPDATE workflow_events SET status = ?1, event_json = ?2 WHERE event_id = ?3",
                rusqlite::params![retryable_status, event_json.to_string(), &event_id],
            )
            .expect("event is marked retryable");

        let events_text = armature()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--status")
            .arg(retryable_status)
            .output()
            .expect("command runs");
        assert!(events_text.status.success());
        let stdout = String::from_utf8_lossy(&events_text.stdout);
        assert!(stdout.contains("error=simulated failure"));

        let retried = run_json(
            armature()
                .arg("retry-event")
                .arg(&file)
                .arg("--store")
                .arg(&store)
                .arg("--event-id")
                .arg(&event_id)
                .arg("--json"),
        );
        assert_eq!(retried["event"]["event_id"], event_id);
        assert_eq!(retried["event"]["status"], "queued");
        assert_eq!(retried["event"]["last_error"], Value::Null);
        last_event_id = event_id;
    }

    let mut event_json = run_json(
        armature()
            .arg("emit")
            .arg(&file)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"retry text"}"#)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    )["event"]
        .clone();
    let text_retry_event_id = event_json["event_id"]
        .as_str()
        .expect("event id")
        .to_string();
    event_json["status"] = serde_json::json!("failed");
    event_json["last_error"] = serde_json::json!("simulated text failure");
    let connection = rusqlite::Connection::open(&store).expect("store opens");
    connection
        .execute(
            "UPDATE workflow_events SET status = ?1, event_json = ?2 WHERE event_id = ?3",
            rusqlite::params!["failed", event_json.to_string(), &text_retry_event_id],
        )
        .expect("event is marked failed");

    let retry_text = armature()
        .arg("retry-event")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .arg("--event-id")
        .arg(&text_retry_event_id)
        .output()
        .expect("command runs");
    assert!(retry_text.status.success());
    let stdout = String::from_utf8_lossy(&retry_text.stdout);
    assert!(stdout.contains(&format!("retried {text_retry_event_id} status=queued")));
    assert!(stdout.contains("pending_events="));

    let retry_again = armature()
        .arg("retry-event")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .arg("--event-id")
        .arg(&last_event_id)
        .output()
        .expect("command runs");
    assert!(!retry_again.status.success());
    let stderr = String::from_utf8_lossy(&retry_again.stderr);
    assert!(stderr.contains("cannot be retried from status queued"));
}

#[test]
fn status_and_overview_surface_workflow_data_summary() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("data-summary.armature");
    std::fs::write(
        &file,
        r#"
machine DataSummaryWorkflow
initial waiting

class WorkItem {
  id string
  status string
}

data {
  seen string[] = []
  count int = 0
  first string = ""
  item WorkItem = { id "W1", status "ready" }
}

event finished {
  runId string
}

state waiting {
  on finished as run {
    assign data.seen = data.seen.append(run.runId)
    assign data.count = list.length(data.seen)
    assign data.first = run.runId
    assign data.item = { id "W1", status "done" }
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    let store = dir.path().join("workflow.sqlite");

    let run = run_json(
        armature()
            .arg("run")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"runId":"worker-1"}"#)
            .arg("--json"),
    );
    assert_eq!(run["status"]["current_state"], "done");
    assert_eq!(
        run["status"]["data_summary"],
        serde_json::json!({
            "count": 1,
            "first": "worker-1",
            "item": {"fields": 2},
            "seen": 1
        })
    );

    let status = run_json(
        armature()
            .arg("status")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(status["data"]["seen"], serde_json::json!(["worker-1"]));
    assert_eq!(
        status["data_summary"],
        serde_json::json!({
            "count": 1,
            "first": "worker-1",
            "item": {"fields": 2},
            "seen": 1
        })
    );

    let overview = run_json(
        armature()
            .arg("overview")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(overview["status"]["data_summary"], status["data_summary"]);

    let status_text = armature()
        .arg("status")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .output()
        .expect("command runs");
    assert!(status_text.status.success());
    let stdout = String::from_utf8_lossy(&status_text.stdout);
    assert!(stdout
        .contains(r#"data summary: {"count":1,"first":"worker-1","item":{"fields":2},"seen":1}"#));

    let compact_status = armature()
        .arg("status")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .arg("--compact")
        .output()
        .expect("command runs");
    assert!(compact_status.status.success());
    let stdout = String::from_utf8_lossy(&compact_status.stdout);
    assert!(stdout.contains("workflow: DataSummaryWorkflow"));
    assert!(stdout.contains("state: done"));
    assert!(stdout.contains("waiting: idle; no queued events or active invocations"));
    assert!(stdout.contains("current blockers: none"));
}

#[test]
fn run_can_use_baml_http_coerce_without_fake_outputs() {
    if !loopback_tests_enabled() {
        eprintln!("skipping explicit BAML HTTP test; set ARMATURE_RUN_LOOPBACK_TESTS=1 to run it");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("baml-http.armature");
    std::fs::write(
        &file,
        r#"
machine BamlHttpWorkflow
initial waiting

data {
  result string = ""
}

event go {
  message string
}

coerce classify(message string) -> string {
  prompt """
Classify this message.

{{ message }}
  """
}

state waiting {
  on go as evt {
    let classification = coerce classify(evt.message)
    assign data.result = classification
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    let store = dir.path().join("workflow.sqlite");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("test server binds");
    let address = listener.local_addr().expect("test server address");
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request accepted");
        let mut request = [0_u8; 4096];
        let bytes_read = std::io::Read::read(&mut stream, &mut request).expect("request reads");
        let request = String::from_utf8_lossy(&request[..bytes_read]);
        assert!(request.starts_with("POST /call/classify "));
        assert!(request.contains(r#""message":"hello""#));
        let body = r#""done""#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        std::io::Write::write_all(&mut stream, response.as_bytes()).expect("response writes");
    });

    let output = run_json(
        armature()
            .arg("run")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--baml-url")
            .arg(format!("http://{address}"))
            .arg("--json"),
    );

    assert_eq!(output["outcome"]["status"], "processed");
    assert_eq!(output["status"]["current_state"], "done");
    assert_eq!(output["status"]["data"]["result"], "done");
    assert_eq!(
        output["status"]["latest_coerce_calls"][0]["function_name"],
        "classify"
    );
    assert_eq!(
        output["status"]["latest_coerce_calls"][0]["parsed_output"],
        "done"
    );
    assert_eq!(
        output["status"]["latest_coerce_calls"][0]["backend"]["kind"],
        "baml_http"
    );
    assert_eq!(
        output["status"]["latest_coerce_calls"][0]["backend"]["runtime_mode"],
        Value::Null
    );
    assert_eq!(output["status"]["baml_runtime"]["mode"], "external_http");
    assert_eq!(output["status"]["baml_runtime"]["status"], "observed");
    assert_eq!(
        output["status"]["baml_runtime"]["url"],
        format!("http://{address}")
    );
    assert!(
        output["status"]["latest_coerce_calls"][0]["backend"]["baml_src_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );
    let stored_step_path: String = rusqlite::Connection::open(&store)
        .expect("store opens")
        .query_row(
            "SELECT step_path FROM coerce_calls WHERE workflow_id = 'BamlHttpWorkflow'",
            [],
            |row| row.get(0),
        )
        .expect("coerce step path reads");
    assert_eq!(stored_step_path, "handler.0");
    let stored_raw_response: String = rusqlite::Connection::open(&store)
        .expect("store opens")
        .query_row(
            "SELECT raw_response_json FROM coerce_calls WHERE workflow_id = 'BamlHttpWorkflow'",
            [],
            |row| row.get(0),
        )
        .expect("coerce raw response reads");
    assert_eq!(stored_raw_response, r#""done""#);
    handle.join().expect("test server joins");
}

#[test]
fn run_uses_generated_stdio_baml_by_default_when_coerce_exists() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_baml_coerce_workflow(&dir, "generated-stdio-baml.armature");
    let store = dir.path().join("workflow.sqlite");
    let (fake_runner, fake_runner_log) = write_fake_generated_stdio_runner(&dir);

    let mut command = armature();
    command_with_fake_generated_stdio_runner(&mut command, &fake_runner, &fake_runner_log);
    let output = run_json(
        command
            .arg("run")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--json"),
    );

    assert_eq!(output["outcome"]["status"], "processed");
    assert_eq!(output["status"]["current_state"], "done");
    assert_eq!(output["status"]["data"]["result"], "generated-stdio");
    assert_eq!(
        output["status"]["latest_coerce_calls"][0]["function_name"],
        "classify"
    );
    assert_eq!(
        output["status"]["latest_coerce_calls"][0]["parsed_output"],
        "generated-stdio"
    );
    assert_eq!(
        output["status"]["latest_coerce_calls"][0]["backend"]["kind"],
        "baml_generated_stdio"
    );
    assert_eq!(
        output["status"]["latest_coerce_calls"][0]["backend"]["runtime_mode"],
        "generated_stdio"
    );
    assert_eq!(output["status"]["baml_runtime"]["mode"], "generated_stdio");
    assert_eq!(output["status"]["baml_runtime"]["status"], "observed");
    assert!(output["status"]["baml_runtime"]["baml_src_hash"]
        .as_str()
        .is_some_and(|hash| hash.starts_with("sha256:")));
    assert!(
        output["status"]["latest_coerce_calls"][0]["backend"]["baml_src_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );
    assert!(
        output["status"]["latest_coerce_calls"][0]["backend"]["runner_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );

    let fake_log =
        std::fs::read_to_string(&fake_runner_log).expect("fake generated stdio log reads");
    assert!(
        fake_log.contains(r#""function_name":"classify""#)
            || fake_log.contains(r#""function":"classify""#)
    );
    assert!(fake_log.contains(r#""message":"hello""#));
    assert!(fake_log.contains(r#""arg_order":["message"]"#));

    let baml_source = std::fs::read_to_string(
        dir.path()
            .join(".armature/build/workflows/BamlHttpWorkflow/baml_src/workflow.baml"),
    )
    .expect("managed BAML source artifact reads");
    assert!(baml_source.contains("function Classify"));
    assert!(baml_source.contains("message: string"));

    let managed_runner =
        std::fs::read_to_string(dir.path().join(
            ".armature/build/workflows/BamlHttpWorkflow/baml_runner/armature-baml-runner.mjs",
        ))
        .expect("managed BAML stdio runner artifact reads");
    assert!(managed_runner.contains("ARMATURE_BAML_STDIO_PROTOCOL_VERSION"));
    assert!(managed_runner.contains("importGeneratedClient"));

    let (backend_json, parsed_output): (String, String) = rusqlite::Connection::open(&store)
        .expect("store opens")
        .query_row(
            "SELECT backend_json, parsed_output_json FROM coerce_calls WHERE workflow_id = 'BamlHttpWorkflow'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("coerce backend and parsed output read");
    assert!(backend_json.contains(r#""kind":"baml_generated_stdio""#));
    assert!(backend_json.contains(r#""runtime_mode":"generated_stdio""#));
    assert!(!backend_json.contains(r#""kind":"fake""#));
    assert_eq!(parsed_output, r#""generated-stdio""#);
}

#[test]
fn run_managed_generated_stdio_runs_baml_generate_and_generated_client() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping managed generated stdio test; node is unavailable");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_baml_coerce_workflow(&dir, "managed-generated-stdio-baml.armature");
    let store = dir.path().join("workflow.sqlite");
    let (fake_baml_cli_dir, fake_baml_generate_log) = write_fake_baml_generate_cli(&dir);

    let mut command = armature();
    command_with_fake_baml_generate(&mut command, &fake_baml_cli_dir, &fake_baml_generate_log);
    let output = run_json(
        command
            .arg("run")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--json"),
    );

    assert_eq!(output["outcome"]["status"], "processed");
    assert_eq!(output["status"]["current_state"], "done");
    assert_eq!(output["status"]["data"]["result"], "generated-client:hello");
    assert_eq!(
        output["status"]["latest_coerce_calls"][0]["backend"]["kind"],
        "baml_generated_stdio"
    );

    let generate_log =
        std::fs::read_to_string(&fake_baml_generate_log).expect("fake baml generate log reads");
    assert!(generate_log.contains(r#""args":["generate","--from""#));
    assert!(generate_log.contains(r#""generator_contains_typescript":true"#));
    assert!(generate_log.contains(r#""workflow_contains_classify":true"#));

    let generator_source = std::fs::read_to_string(
        dir.path()
            .join(".armature/build/workflows/BamlHttpWorkflow/baml_src/generators.baml"),
    )
    .expect("managed BAML generator source artifact reads");
    assert!(generator_source.contains(r#"output_dir "../baml_runner""#));
}

#[test]
fn run_generated_stdio_can_use_codex_oauth_auth_source() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_baml_coerce_workflow(&dir, "generated-stdio-codex-oauth.armature");
    let store = dir.path().join("workflow.sqlite");
    let (fake_runner, fake_runner_log) = write_fake_generated_stdio_runner(&dir);
    let codex_home = dir.path().join("codex-home");
    std::fs::create_dir_all(&codex_home).expect("codex home writes");
    std::fs::write(
        codex_home.join("auth.json"),
        r#"{"tokens":{"access_token":"codex-oauth-test-token"}}"#,
    )
    .expect("codex auth writes");

    let mut command = armature();
    command_with_fake_generated_stdio_runner(&mut command, &fake_runner, &fake_runner_log);
    let output = run_json(
        command
            .env("CODEX_HOME", &codex_home)
            .arg("run")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--baml-auth")
            .arg("codex-oauth")
            .arg("--json"),
    );

    assert_eq!(output["outcome"]["status"], "processed");
    assert_eq!(output["status"]["data"]["result"], "generated-stdio");

    let fake_log =
        std::fs::read_to_string(&fake_runner_log).expect("fake generated stdio log reads");
    assert!(fake_log.contains(r#""codex_oauth_token_present":true"#));
    assert!(fake_log.contains(r#""codex_oauth_base_url":"https://chatgpt.com/backend-api/codex""#));

    let baml_source = std::fs::read_to_string(
        dir.path()
            .join(".armature/build/workflows/BamlHttpWorkflow/baml_src/workflow.baml"),
    )
    .expect("managed BAML source artifact reads");
    assert!(baml_source.contains(r#"provider "openai-responses""#));
    assert!(baml_source.contains("api_key env.ARMATURE_CODEX_OAUTH_ACCESS_TOKEN"));
    assert!(baml_source.contains("base_url env.ARMATURE_CODEX_OAUTH_BASE_URL"));
    assert!(baml_source.contains("store false"));
    assert!(baml_source.contains("stream true"));
    assert!(baml_source.contains("instructions "));
}

#[test]
fn run_baml_url_bypasses_managed_baml_cli_even_when_fake_is_on_path() {
    if !loopback_tests_enabled() {
        eprintln!("skipping explicit BAML HTTP test; set ARMATURE_RUN_LOOPBACK_TESTS=1 to run it");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_baml_coerce_workflow(&dir, "external-baml.armature");
    let store = dir.path().join("workflow.sqlite");
    let (fake_baml_bin_dir, fake_baml_log) = write_fake_baml_cli(&dir);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("test server binds");
    let address = listener.local_addr().expect("test server address");
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request accepted");
        let mut request = [0_u8; 4096];
        let bytes_read = std::io::Read::read(&mut stream, &mut request).expect("request reads");
        let request = String::from_utf8_lossy(&request[..bytes_read]);
        assert!(request.starts_with("POST /call/classify "));
        assert!(request.contains(r#""message":"hello""#));
        let body = r#""external""#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        std::io::Write::write_all(&mut stream, response.as_bytes()).expect("response writes");
    });

    let mut command = armature();
    command_with_fake_baml_cli(&mut command, &fake_baml_bin_dir, &fake_baml_log);
    let output = run_json(
        command
            .arg("run")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--baml-url")
            .arg(format!("http://{address}"))
            .arg("--json"),
    );

    assert_eq!(output["outcome"]["status"], "processed");
    assert_eq!(output["status"]["current_state"], "done");
    assert_eq!(output["status"]["data"]["result"], "external");
    assert_eq!(
        output["status"]["latest_coerce_calls"][0]["backend"]["kind"],
        "baml_http"
    );
    assert_eq!(
        output["status"]["latest_coerce_calls"][0]["backend"]["runtime_mode"],
        Value::Null
    );
    assert_eq!(output["status"]["baml_runtime"]["mode"], "external_http");
    assert_eq!(output["status"]["baml_runtime"]["status"], "observed");
    assert_eq!(
        output["status"]["baml_runtime"]["url"],
        format!("http://{address}")
    );
    assert!(
        !fake_baml_log.exists()
            || std::fs::read_to_string(&fake_baml_log)
                .expect("fake baml log reads")
                .is_empty()
    );
    handle.join().expect("test server joins");
}

#[test]
fn run_rejects_duplicate_fake_outputs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("fake-duplicates.armature");
    std::fs::write(
        &file,
        r#"
machine FakeDuplicates
initial waiting

event go {}

coerce classify() -> string {
  prompt "Classify"
}

state waiting {
  on go {
    let result = coerce classify()
    stay
  }
}
"#,
    )
    .expect("workflow writes");

    let output = armature()
        .arg("run")
        .arg(&file)
        .arg("--event")
        .arg("go")
        .arg("--fake-coerce-output")
        .arg(r#"classify="one""#)
        .arg("--fake-coerce-output")
        .arg(r#"classify="two""#)
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("duplicate fake output `classify`"));
}

#[test]
fn run_rejects_fake_output_names_with_whitespace() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("fake-whitespace.armature");
    std::fs::write(
        &file,
        r#"
machine FakeWhitespace
initial waiting

event go {}

coerce classify() -> string {
  prompt "Classify"
}

state waiting {
  on go {
    let result = coerce classify()
    stay
  }
}
"#,
    )
    .expect("workflow writes");

    let output = armature()
        .arg("run")
        .arg(&file)
        .arg("--event")
        .arg("go")
        .arg("--fake-coerce-output")
        .arg(r#"classify result="one""#)
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(r#"invalid fake output `classify result="one"`"#));
}

#[test]
fn run_redacts_baml_http_raw_response_by_enterprise_policy_default() {
    if !loopback_tests_enabled() {
        eprintln!("skipping explicit BAML HTTP test; set ARMATURE_RUN_LOOPBACK_TESTS=1 to run it");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("baml-http.armature");
    std::fs::write(
        &file,
        r#"
machine BamlHttpWorkflow
initial waiting

data {
  result string = ""
}

event go {
  message string
}

coerce classify(message string) -> string {
  prompt """
Classify this message.

{{ message }}
  """
}

state waiting {
  on go as evt {
    let classification = coerce classify(evt.message)
    assign data.result = classification
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    let store = dir.path().join("workflow.sqlite");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("test server binds");
    let address = listener.local_addr().expect("test server address");
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request accepted");
        let mut request = [0_u8; 4096];
        let _bytes_read = std::io::Read::read(&mut stream, &mut request).expect("request reads");
        let body = r#""done""#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        std::io::Write::write_all(&mut stream, response.as_bytes()).expect("response writes");
    });
    let policy = dir.path().join("policy.json");
    std::fs::write(
        &policy,
        format!(
            r#"{{
  "mode": "enterprise",
  "allowed_capabilities": ["baml.coerce"],
  "allow_baml_network": true,
  "allowed_baml_urls": ["http://{address}"]
}}"#
        ),
    )
    .expect("policy writes");

    let output = run_json(
        armature()
            .arg("run")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--baml-url")
            .arg(format!("http://{address}"))
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );

    assert_eq!(output["outcome"]["status"], "processed");
    assert_eq!(output["status"]["data"]["result"], "done");
    let (raw_response, parsed_output): (String, String) = rusqlite::Connection::open(&store)
        .expect("store opens")
        .query_row(
            "SELECT raw_response_json, parsed_output_json FROM coerce_calls WHERE workflow_id = 'BamlHttpWorkflow'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("coerce response fields read");
    assert_eq!(
        serde_json::from_str::<Value>(&raw_response).expect("raw response parses"),
        serde_json::json!({"redacted": true, "reason": "policy"})
    );
    assert_eq!(parsed_output, r#""done""#);
    handle.join().expect("test server joins");
}

#[test]
fn status_text_surfaces_latest_coerce_failures() {
    if !loopback_tests_enabled() {
        eprintln!("skipping explicit BAML HTTP test; set ARMATURE_RUN_LOOPBACK_TESTS=1 to run it");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("baml-http-failure.armature");
    std::fs::write(
        &file,
        r#"
machine BamlHttpFailureWorkflow
initial waiting

data {
  result string = ""
}

event go {
  message string
}

coerce classify(message string) -> string {
  prompt """
Classify this message.

{{ message }}
  """
}

state waiting {
  on go as evt {
    let classification = coerce classify(evt.message)
    assign data.result = classification
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    let store = dir.path().join("workflow.sqlite");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("test server binds");
    let address = listener.local_addr().expect("test server address");
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request accepted");
        let mut request = [0_u8; 4096];
        let _bytes_read = std::io::Read::read(&mut stream, &mut request).expect("request reads");
        let body = r#"{"error":"model unavailable"}"#;
        let response = format!(
            "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        std::io::Write::write_all(&mut stream, response.as_bytes()).expect("response writes");
    });

    let run = armature()
        .arg("run")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .arg("--event")
        .arg("go")
        .arg("--payload")
        .arg(r#"{"message":"hello"}"#)
        .arg("--baml-url")
        .arg(format!("http://{address}"))
        .arg("--json")
        .output()
        .expect("command runs");
    assert!(!run.status.success());
    handle.join().expect("test server joins");

    let status = run_json(
        armature()
            .arg("status")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(
        status["latest_coerce_failures"][0]["function_name"],
        "classify"
    );
    assert_eq!(status["latest_coerce_failures"][0]["status"], "failed");
    assert_eq!(status["latest_coerce_failures"][0]["http_status"], 503);
    assert_eq!(
        status["current_coerce_failure"]["function_name"],
        "classify"
    );
    assert!(status["current_blockers"][0]
        .as_str()
        .is_some_and(|blocker| blocker.contains("coerce `classify` failed")));

    let status_text = armature()
        .arg("status")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .output()
        .expect("command runs");
    assert!(status_text.status.success());
    let stdout = String::from_utf8_lossy(&status_text.stdout);
    assert!(stdout.contains("current blockers:"));
    assert!(stdout.contains("current coerce failure: classify Failed http=Some(503)"));
    assert!(stdout.contains("latest coerce failures (history):"));
    assert!(stdout.contains("classify Failed http=Some(503)"));
}

#[test]
fn retry_success_clears_current_coerce_failure_but_keeps_history() {
    if !loopback_tests_enabled() {
        eprintln!("skipping explicit BAML HTTP test; set ARMATURE_RUN_LOOPBACK_TESTS=1 to run it");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("coerce-retry.armature");
    std::fs::write(
        &file,
        r#"
machine CoerceRetry
initial waiting

event go {
  message string
}

coerce classify(message string) -> string {
  prompt """
classify
  """
}

state waiting {
  on go as evt {
    let classification = coerce classify(evt.message)
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    let store = dir.path().join("workflow.sqlite");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("test server binds");
    let address = listener.local_addr().expect("test server address");
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("request accepted");
        let mut request = [0_u8; 4096];
        let _ = std::io::Read::read(&mut stream, &mut request).expect("request reads");
        let body = "model unavailable";
        let response = format!(
            "HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        std::io::Write::write_all(&mut stream, response.as_bytes()).expect("response writes");
    });

    let failed = armature()
        .arg("run")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .arg("--event")
        .arg("go")
        .arg("--payload")
        .arg(r#"{"message":"hello"}"#)
        .arg("--baml-url")
        .arg(format!("http://{address}"))
        .arg("--json")
        .output()
        .expect("command runs");
    assert!(!failed.status.success());
    handle.join().expect("test server joins");

    let failed_events = run_json(
        armature()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--status")
            .arg("failed")
            .arg("--json"),
    );
    let event_id = failed_events["events"][0]["event_id"]
        .as_str()
        .expect("event id");

    let retry = run_json(
        armature()
            .arg("retry-event")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--event-id")
            .arg(event_id)
            .arg("--json"),
    );
    assert_eq!(retry["event"]["status"], "queued");
    assert!(retry["status"]["current_coerce_failure"].is_null());
    assert_eq!(
        retry["status"]["current_blockers"],
        Value::Array(Vec::new())
    );
    assert_eq!(retry["status"]["pending_events"], 1);

    let processed = run_json(
        armature()
            .arg("run")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--fake-coerce-output")
            .arg(r#"classify="ok""#)
            .arg("--json"),
    );
    assert_eq!(processed["outcome"]["status"], "processed");
    assert!(processed["status"]["current_coerce_failure"].is_null());
    assert_eq!(
        processed["status"]["current_blockers"],
        Value::Array(Vec::new())
    );
    assert_eq!(
        processed["status"]["latest_coerce_failures"][0]["function_name"],
        "classify"
    );

    let status_text = armature()
        .arg("status")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .output()
        .expect("command runs");
    assert!(status_text.status.success());
    let stdout = String::from_utf8_lossy(&status_text.stdout);
    assert!(stdout.contains("current blockers: none"));
    assert!(stdout.contains("current coerce failure: none"));
    assert!(stdout.contains("latest coerce failures (history):"));
}

#[test]
fn run_rejects_baml_http_url_disallowed_by_enterprise_policy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("baml-http.armature");
    std::fs::write(
        &file,
        r#"
machine BamlHttpWorkflow
initial waiting

event go {
  message string
}

coerce classify(message string) -> string {
  prompt """
Classify this message.

{{ message }}
  """
}

state waiting {
  on go as evt {
    let classification = coerce classify(evt.message)
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    let policy = dir.path().join("policy.json");
    std::fs::write(
        &policy,
        r#"{
  "mode": "enterprise",
  "allowed_capabilities": ["baml.coerce"],
  "allow_baml_network": true,
  "allowed_baml_urls": ["http://127.0.0.1:2024"]
}"#,
    )
    .expect("policy writes");

    let output = armature()
        .arg("run")
        .arg(&file)
        .arg("--event")
        .arg("go")
        .arg("--payload")
        .arg(r#"{"message":"hello"}"#)
        .arg("--baml-url")
        .arg("http://127.0.0.1:2025")
        .arg("--policy")
        .arg(&policy)
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("BAML HTTP URL `http://127.0.0.1:2025` is not allowed"));
}

#[test]
fn overview_reports_validation_failures_without_runtime_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("invalid.armature");
    std::fs::write(
        &file,
        r#"machine
initial waiting
"#,
    )
    .expect("workflow writes");

    let overview = run_json(armature().arg("overview").arg(&file).arg("--json"));

    assert_eq!(overview["validation"]["ok"], false);
    assert!(overview["status"].is_null());
    assert!(overview["validation"]["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|diagnostic| diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("expected machine name"))));

    let output = armature()
        .arg("overview")
        .arg(&file)
        .output()
        .expect("command runs");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("validation: failed"));
    assert!(stdout.contains("runtime: unavailable"));
}

#[test]
fn overview_reports_adapter_manifest_validation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "empty-adapter",
  "version": "0.1.0",
  "effects": {},
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let overview = run_json(
        armature()
            .arg("overview")
            .arg(&file)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--json"),
    );

    assert_eq!(overview["validation"]["ok"], false);
    assert_eq!(overview["status"]["current_state"], "waiting");
    assert!(overview["validation"]["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|diagnostic| diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("effect `send` is not declared"))));

    let overview_text = armature()
        .arg("overview")
        .arg(&file)
        .arg("--adapter-manifest")
        .arg(&manifest)
        .output()
        .expect("command runs");
    assert!(overview_text.status.success());
    let stdout = String::from_utf8_lossy(&overview_text.stdout);
    assert!(stdout.contains("waiting: validation failed; inspect diagnostics above"));
}

#[test]
fn status_validates_adapter_manifest_contracts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "empty-adapter",
  "version": "0.1.0",
  "effects": {},
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = armature()
        .arg("status")
        .arg(&file)
        .arg("--adapter-manifest")
        .arg(&manifest)
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("workflow contract validation failed"));
    assert!(stderr.contains("effect `send` is not declared"));
}

#[test]
fn events_and_log_accept_validation_context_flags() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let store = dir.path().join("workflow.sqlite");
    let agent_file = dir.path().join("agents.json");
    let policy = dir.path().join("policy.json");
    std::fs::write(
        &policy,
        r#"{
  "mode": "local",
  "allowed_capabilities": ["message_agents"],
  "denied_capabilities": []
}"#,
    )
    .expect("policy writes");

    run_json(
        armature()
            .arg("emit")
            .arg(&file)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );

    let events = run_json(
        armature()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--fixture-agent-file")
            .arg(&agent_file)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(events["events"][0]["event_type"], "go");

    let log = run_json(
        armature()
            .arg("log")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--fixture-agent-file")
            .arg(&agent_file)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(log["records"], Value::Array(Vec::new()));
}

#[test]
fn events_and_log_reject_unbounded_limits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let excessive_limit = usize::MAX.to_string();

    for command_name in ["events", "log"] {
        let output = armature()
            .arg(command_name)
            .arg(&file)
            .arg("--limit")
            .arg(&excessive_limit)
            .output()
            .expect("command runs");

        assert!(!output.status.success(), "{command_name} should fail");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(&format!("limit for {command_name} must be <= 10000")));
    }
}

#[test]
fn events_and_log_validate_adapter_manifest_contracts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "empty-adapter",
  "version": "0.1.0",
  "effects": {},
  "events": {}
}"#,
    )
    .expect("manifest writes");

    for command_name in ["events", "log"] {
        let output = armature()
            .arg(command_name)
            .arg(&file)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .output()
            .expect("command runs");

        assert!(!output.status.success(), "{command_name} should fail");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("workflow contract validation failed"));
        assert!(stderr.contains("effect `send` is not declared"));
    }
}

#[test]
fn status_and_overview_validate_policy_documents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let policy = dir.path().join("bad-policy.json");
    std::fs::write(
        &policy,
        r#"{
  "mode": "local",
  "allowed_capabilities": ["message_agents", "message_agents"],
  "denied_capabilities": []
}"#,
    )
    .expect("policy writes");

    let status = armature()
        .arg("status")
        .arg(&file)
        .arg("--policy")
        .arg(&policy)
        .output()
        .expect("command runs");
    assert!(!status.status.success());
    let stderr = String::from_utf8_lossy(&status.stderr);
    assert!(stderr.contains("policy document validation failed"));
    assert!(stderr.contains("repeats capability `message_agents`"));

    for command_name in ["events", "log"] {
        let output = armature()
            .arg(command_name)
            .arg(&file)
            .arg("--policy")
            .arg(&policy)
            .output()
            .expect("command runs");

        assert!(!output.status.success(), "{command_name} should fail");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("policy document validation failed"));
        assert!(stderr.contains("repeats capability `message_agents`"));
    }

    let overview = run_json(
        armature()
            .arg("overview")
            .arg(&file)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(overview["validation"]["ok"], false);
    assert_eq!(overview["status"]["current_state"], "waiting");
    assert!(overview["validation"]["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|diagnostic| diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("repeats capability `message_agents`"))));
}

#[test]
fn status_and_overview_do_not_read_file_backed_adapter_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("plan-only.armature");
    std::fs::write(
        &file,
        r#"
machine PlanOnly
initial waiting

data {
  snapshot string? = nil
}

capability plan = adapter("implementationPlan")

state waiting {
  entry {
    let text = plan.snapshot()
    assign data.snapshot = text
  }
}
"#,
    )
    .expect("workflow writes");
    let missing_plan = dir.path().join("missing-plan.json");

    let status = run_json(
        armature()
            .arg("status")
            .arg(&file)
            .arg("--plan-file")
            .arg(&missing_plan)
            .arg("--json"),
    );
    assert_eq!(status["current_state"], "waiting");
    assert_eq!(status["data"]["snapshot"], Value::Null);

    let overview = run_json(
        armature()
            .arg("overview")
            .arg(&file)
            .arg("--plan-file")
            .arg(&missing_plan)
            .arg("--json"),
    );
    assert_eq!(overview["validation"]["ok"], true);
    assert_eq!(overview["status"]["current_state"], "waiting");

    let events = run_json(
        armature()
            .arg("events")
            .arg(&file)
            .arg("--plan-file")
            .arg(&missing_plan)
            .arg("--json"),
    );
    assert_eq!(events["workflow_id"], "PlanOnly");
    assert_eq!(events["events"], Value::Array(Vec::new()));

    let log = run_json(
        armature()
            .arg("log")
            .arg(&file)
            .arg("--plan-file")
            .arg(&missing_plan)
            .arg("--json"),
    );
    assert_eq!(log["workflow_id"], "PlanOnly");
    assert_eq!(log["records"], Value::Array(Vec::new()));

    assert!(
        !missing_plan.exists(),
        "inspection commands must not create or read backing files"
    );
}

#[test]
fn run_rejects_manifest_missing_effect_before_dispatch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let store = dir.path().join("workflow.sqlite");
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "empty-adapter",
  "version": "0.1.0",
  "effects": {},
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = armature()
        .arg("run")
        .arg(&file)
        .arg("--event")
        .arg("go")
        .arg("--payload")
        .arg(r#"{"message":"hello"}"#)
        .arg("--store")
        .arg(&store)
        .arg("--adapter-manifest")
        .arg(&manifest)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("effect `send` is not declared"));
    assert!(stderr.contains("workflow.armature:13:5"));
    assert!(!store.exists());
}

#[test]
fn validate_uses_adapter_manifest_for_static_effect_checks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "empty-adapter",
  "version": "0.1.0",
  "effects": {},
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = armature()
        .arg("validate")
        .arg(&file)
        .arg("--adapter-manifest")
        .arg(&manifest)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout is json");
    assert_eq!(stdout["ok"], false);
    let diagnostics = stdout["diagnostics"].as_array().expect("diagnostics");
    let effect_diagnostic = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("effect `send` is not declared"))
        })
        .expect("effect diagnostic exists");
    assert!(effect_diagnostic["span"]["file"]
        .as_str()
        .expect("span file")
        .ends_with("workflow.armature"));
    assert_eq!(effect_diagnostic["span"]["start_line"], 13);
    assert_eq!(effect_diagnostic["span"]["start_column"], 5);
}

#[test]
fn validate_accepts_file_backed_adapter_flags_for_static_effect_checks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let agents = dir.path().join("agents.json");

    let output = run_json(
        armature()
            .arg("validate")
            .arg(&file)
            .arg("--fixture-agent-file")
            .arg(&agents)
            .arg("--json"),
    );

    assert_eq!(output["ok"], true);
    assert_eq!(output["diagnostics"], Value::Array(Vec::new()));
}

#[test]
fn validate_reports_adapter_manifest_input_shape_mismatches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "message-adapter",
  "version": "0.1.0",
  "effects": {
    "send": {
      "category": "message",
      "required_capabilities": ["message_agents"],
      "input": {
        "type": "record",
        "fields": [
          {"name": "message", "schema": {"type": "string"}}
        ]
      },
      "output": {"type": "json"},
      "idempotent": true,
      "failure_categories": ["delivery_failed"],
      "model": {"kind": "opaque"}
    }
  },
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = armature()
        .arg("validate")
        .arg(&file)
        .arg("--adapter-manifest")
        .arg(&manifest)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout is json");
    assert_eq!(stdout["ok"], false);
    let diagnostics = stdout["diagnostics"].as_array().expect("diagnostics");
    assert!(diagnostics.iter().any(|diagnostic| diagnostic["message"]
        .as_str()
        .is_some_and(|message| message.contains("request args"))));
}

#[test]
fn validate_applies_local_policy_as_warnings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "message-adapter",
  "version": "0.1.0",
  "effects": {
    "send": {
      "category": "message",
      "required_capabilities": ["message_agents"],
      "input": {"type": "json"},
      "output": {"type": "json"},
      "idempotent": true,
      "failure_categories": ["delivery_failed"],
      "model": {"kind": "opaque"}
    }
  },
  "events": {}
}"#,
    )
    .expect("manifest writes");
    let policy = dir.path().join("policy.json");
    std::fs::write(
        &policy,
        r#"{
  "mode": "local",
  "allowed_capabilities": [],
  "denied_capabilities": []
}"#,
    )
    .expect("policy writes");

    let output = run_json(
        armature()
            .arg("validate")
            .arg(&file)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );

    assert_eq!(output["ok"], true);
    let diagnostic = output["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .find(|diagnostic| {
            diagnostic["message"]
                .as_str()
                .is_some_and(|message| message.contains("requires capability `message_agents`"))
        })
        .expect("policy diagnostic exists");
    assert_eq!(diagnostic["severity"], "Warning");
    assert_eq!(diagnostic["span"]["start_line"], 13);
    assert!(diagnostic["message"].as_str().is_some_and(
        |message| message.contains("Fix: add `message_agents` to allowed_capabilities")
    ));
}

#[test]
fn validate_applies_enterprise_policy_as_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "message-adapter",
  "version": "0.1.0",
  "effects": {
    "send": {
      "category": "message",
      "required_capabilities": ["message_agents"],
      "input": {"type": "json"},
      "output": {"type": "json"},
      "idempotent": true,
      "failure_categories": ["delivery_failed"],
      "model": {"kind": "opaque"}
    }
  },
  "events": {}
}"#,
    )
    .expect("manifest writes");
    let policy = dir.path().join("policy.json");
    std::fs::write(
        &policy,
        r#"{
  "mode": "enterprise",
  "allowed_capabilities": [],
  "denied_capabilities": []
}"#,
    )
    .expect("policy writes");

    let output = armature()
        .arg("validate")
        .arg(&file)
        .arg("--adapter-manifest")
        .arg(&manifest)
        .arg("--policy")
        .arg(&policy)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout is json");
    assert_eq!(stdout["ok"], false);
    assert!(stdout["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(
            |diagnostic| diagnostic["message"].as_str().is_some_and(|message| message
                .contains("requires capability `message_agents`")
                && message.contains("Fix: add `message_agents` to allowed_capabilities"))
        ));
}

#[test]
fn validate_reports_adapter_manifest_diagnostics() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "bad-adapter",
  "version": "",
  "effects": {
    "send": {
      "category": "message",
      "required_capabilities": ["message_agents", "message_agents", "bad capability"],
      "input": {"type": "ref", "name": "MissingPayload"},
      "output": {"type": "json"},
      "idempotent": false,
      "failure_categories": [],
      "model": {"kind": "opaque"}
    }
  },
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = armature()
        .arg("validate")
        .arg(&file)
        .arg("--adapter-manifest")
        .arg(&manifest)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout is json");
    assert_eq!(stdout["ok"], false);
    let messages = stdout["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter_map(|diagnostic| diagnostic["message"].as_str())
        .collect::<Vec<_>>();
    assert!(messages
        .iter()
        .any(|message| message.contains("version must not be empty")));
    assert!(messages
        .iter()
        .any(|message| message.contains("repeats required capability")));
    assert!(messages.iter().any(|message| {
        message.contains(
            "required capability `bad capability` contains whitespace or control characters",
        )
    }));
    assert!(messages
        .iter()
        .any(|message| message.contains("references unknown type")));
    assert!(messages
        .iter()
        .any(|message| message.contains("must be idempotent")));
}

#[test]
fn validate_adapter_accepts_manifest_without_workflow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "message-adapter",
  "version": "0.1.0",
  "effects": {
    "send": {
      "category": "message",
      "required_capabilities": ["message_agents"],
      "input": {"type": "json"},
      "output": {"type": "json"},
      "idempotent": true,
      "failure_categories": ["delivery_failed"],
      "model": {"kind": "opaque"}
    }
  },
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = run_json(
        armature()
            .arg("validate-adapter")
            .arg(&manifest)
            .arg("--json"),
    );

    assert_eq!(output["ok"], true);
    assert_eq!(output["diagnostics"], Value::Array(Vec::new()));
}

#[test]
fn validate_adapter_reports_manifest_token_diagnostics() {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "message-adapter",
  "version": "0.1.0",
  "effects": {
    "send": {
      "category": "message",
      "required_capabilities": ["message_agents", "message_agents", "bad capability"],
      "input": {"type": "json"},
      "output": {"type": "json"},
      "idempotent": true,
      "failure_categories": ["delivery_failed", "delivery_failed", "bad category"],
      "model": {
        "kind": "nondeterministic_outcome",
        "values": ["ok", "ok", "needs review"]
      }
    }
  },
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = armature()
        .arg("validate-adapter")
        .arg(&manifest)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout is json");
    assert_eq!(stdout["ok"], false);
    let messages = stdout["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter_map(|diagnostic| diagnostic["message"].as_str())
        .collect::<Vec<_>>();
    assert!(messages
        .iter()
        .any(|message| message.contains("repeats required capability `message_agents`")));
    assert!(messages.iter().any(|message| {
        message.contains(
            "required capability `bad capability` contains whitespace or control characters",
        )
    }));
    assert!(messages
        .iter()
        .any(|message| message.contains("repeats failure category `delivery_failed`")));
    assert!(messages.iter().any(|message| {
        message
            .contains("failure category `bad category` contains whitespace or control characters")
    }));
    assert!(messages
        .iter()
        .any(|message| message.contains("repeats nondeterministic model value `ok`")));
    assert!(messages.iter().any(|message| {
        message.contains(
            "nondeterministic model value `needs review` contains whitespace or control characters",
        )
    }));
}

#[test]
fn validate_adapter_reports_cross_manifest_duplicates() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = dir.path().join("first.json");
    let second = dir.path().join("second.json");
    let manifest = r#"{
  "name": "message-adapter",
  "version": "0.1.0",
  "effects": {
    "send": {
      "category": "message",
      "required_capabilities": ["message_agents"],
      "input": {"type": "json"},
      "output": {"type": "json"},
      "idempotent": true,
      "failure_categories": [],
      "model": {"kind": "opaque"}
    }
  },
  "events": {}
}"#;
    std::fs::write(&first, manifest).expect("first manifest writes");
    std::fs::write(
        &second,
        manifest.replace("message-adapter", "duplicate-adapter"),
    )
    .expect("second manifest writes");

    let output = armature()
        .arg("validate-adapter")
        .arg(&first)
        .arg(&second)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout is json");
    assert_eq!(stdout["ok"], false);
    assert!(stdout["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|diagnostic| diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("adapter effect `send` is declared by both"))));
}

#[test]
fn validate_adapter_requires_at_least_one_manifest() {
    let output = armature()
        .arg("validate-adapter")
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires at least one manifest path"));
}

#[test]
fn validate_policy_accepts_policy_without_workflow() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root");
    let policy = repo_root.join("examples/policies/spec-implementation.enterprise-policy.json");

    let output = run_json(armature().arg("validate-policy").arg(&policy).arg("--json"));

    assert_eq!(output["ok"], true);
    assert_eq!(output["diagnostics"], Value::Array(Vec::new()));
}

#[test]
fn validate_accepts_template_with_local_file_backed_policy() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root");
    let workflow = repo_root.join("examples/templates/simple-agent-supervisor.armature");
    let policy = repo_root.join("examples/policies/local-file-backed.policy.json");
    let agents = repo_root.join("target/tmp/agents.json");

    let output = run_json(
        armature()
            .arg("validate")
            .arg(&workflow)
            .arg("--fixture-agent-file")
            .arg(&agents)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );

    assert_eq!(output["ok"], true);
    assert_eq!(output["diagnostics"], Value::Array(Vec::new()));
}

#[test]
fn validate_policy_reports_policy_diagnostics() {
    let dir = tempfile::tempdir().expect("tempdir");
    let policy = dir.path().join("policy.json");
    std::fs::write(
        &policy,
        r#"{
  "mode": "enterprise",
  "allowed_capabilities": ["message_agents", "message_agents", "askHuman", "bad capability"],
  "denied_capabilities": ["askHuman", ""]
}"#,
    )
    .expect("policy writes");

    let output = armature()
        .arg("validate-policy")
        .arg(&policy)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout is json");
    assert_eq!(stdout["ok"], false);
    let messages = stdout["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter_map(|diagnostic| diagnostic["message"].as_str())
        .collect::<Vec<_>>();
    assert!(messages
        .iter()
        .any(|message| message.contains("repeats capability `message_agents`")));
    assert!(messages
        .iter()
        .any(|message| message.contains("contains an empty capability")));
    assert!(messages.iter().any(|message| {
        message.contains("capability `bad capability` contains whitespace or control characters")
    }));
    assert!(messages.iter().any(|message| {
        message
            .contains("capability `askHuman` in both allowed_capabilities and denied_capabilities")
    }));
}

#[test]
fn validate_policy_requires_at_least_one_policy() {
    let output = armature()
        .arg("validate-policy")
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires at least one policy path"));
}

#[test]
fn run_surfaces_manifest_required_capabilities_in_status_and_log() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    std::fs::write(
        &file,
        workflow_source().replace(
            r#"agent director = thread("director")"#,
            r#"agent director = adapter("director")"#,
        ),
    )
    .expect("workflow rewrites");
    let store = dir.path().join("workflow.sqlite");
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "message-adapter",
  "version": "0.1.0",
  "effects": {
    "send": {
      "category": "message",
      "required_capabilities": ["message_agents"],
      "input": {"type": "json"},
      "output": {"type": "json"},
      "idempotent": true,
      "failure_categories": ["delivery_failed"],
      "model": {"kind": "opaque"}
    }
  },
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let run = run_json(
        armature()
            .arg("run")
            .arg(&file)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"hello"}"#)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--json"),
    );
    assert_eq!(
        run["status"]["recent_effects"][0]["required_capabilities"],
        serde_json::json!(["message_agents"])
    );

    let overview_text = armature()
        .arg("overview")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .output()
        .expect("command runs");
    assert!(overview_text.status.success());
    let stdout = String::from_utf8_lossy(&overview_text.stdout);
    assert!(stdout.contains("requires=message_agents"));
    assert!(stdout.contains("waiting: idle; no queued events or active invocations"));

    let log = run_json(
        armature()
            .arg("log")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(log["records"][0]["type"], "effect");
    assert_eq!(
        log["records"][0]["outcome"]["required_capabilities"],
        serde_json::json!(["message_agents"])
    );
}

#[test]
fn validate_accepts_spec_fixture_with_adapter_manifest() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root");
    let workflow = repo_root.join("examples/workflows/spec-implementation.armature");
    let manifest = repo_root.join("examples/adapters/spec-implementation.fake-adapter.json");

    let output = run_json(
        armature()
            .arg("validate")
            .arg(&workflow)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--json"),
    );

    assert_eq!(output["ok"], true);
    assert_eq!(output["diagnostics"], Value::Array(Vec::new()));
}

#[test]
fn validate_accepts_simple_supervisor_fixture_with_adapter_manifest() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root");
    let workflow = repo_root.join("examples/workflows/simple-supervisor.armature");
    let manifest = repo_root.join("examples/adapters/spec-implementation.fake-adapter.json");

    let output = run_json(
        armature()
            .arg("validate")
            .arg(&workflow)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--json"),
    );

    assert_eq!(output["ok"], true);
    assert_eq!(output["diagnostics"], Value::Array(Vec::new()));
}

#[test]
fn emit_rejects_invalid_payload_before_enqueueing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let store = dir.path().join("workflow.sqlite");

    let output = armature()
        .arg("emit")
        .arg(&file)
        .arg("--event")
        .arg("go")
        .arg("--payload")
        .arg(r#"{"message":42}"#)
        .arg("--store")
        .arg(&store)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "payload does not match schema for event `go`: $.message expected string, got int"
    ));
}

#[test]
fn emit_accepts_adapter_manifest_event_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let store = dir.path().join("workflow.sqlite");
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "events-adapter",
  "version": "0.1.0",
  "types": {
    "Finished": {
      "type": "record",
      "fields": [
        {"name": "name", "schema": {"type": "string"}}
      ]
    }
  },
  "effects": {},
  "events": {
    "finished": {"type": "ref", "name": "Finished"}
  }
}"#,
    )
    .expect("manifest writes");

    let emitted = run_json(
        armature()
            .arg("emit")
            .arg(&file)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"name":"worker-1"}"#)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--json"),
    );

    assert_eq!(emitted["event"]["event_type"], "finished");
    assert_eq!(emitted["event"]["payload"]["name"], "worker-1");
    assert_eq!(emitted["status"]["pending_events"], 1);
}

#[test]
fn emit_requires_adapter_event_schema_when_event_names_overlap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let store = dir.path().join("workflow.sqlite");
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "events-adapter",
  "version": "0.1.0",
  "effects": {},
  "events": {
    "go": {
      "type": "record",
      "fields": [
        {"name": "name", "schema": {"type": "string"}}
      ]
    }
  }
}"#,
    )
    .expect("manifest writes");

    let output = armature()
        .arg("emit")
        .arg(&file)
        .arg("--event")
        .arg("go")
        .arg("--payload")
        .arg(r#"{"message":"hello"}"#)
        .arg("--store")
        .arg(&store)
        .arg("--adapter-manifest")
        .arg(&manifest)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "payload does not match schema for event `go`: $.message is not declared in schema"
    ));
}

#[test]
fn emit_validates_policy_documents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let policy = dir.path().join("policy.json");
    std::fs::write(
        &policy,
        r#"{
  "mode": "local",
  "allowed_capabilities": ["bad capability"]
}"#,
    )
    .expect("policy writes");

    let output = armature()
        .arg("emit")
        .arg(&file)
        .arg("--event")
        .arg("go")
        .arg("--payload")
        .arg(r#"{"message":"hello"}"#)
        .arg("--policy")
        .arg(&policy)
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("capability `bad capability` contains whitespace or control characters"));
}

#[test]
fn emit_model_outputs_tla_module() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);

    let output = armature()
        .arg("emit-model")
        .arg(file)
        .arg("--target")
        .arg("tla")
        .output()
        .expect("command runs");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("---- MODULE Armature_CliWorkflow ----"));
    assert!(stdout.contains(r#"state = "waiting""#));
    assert!(stdout.contains("active = [agent \\in AgentsWithMax |-> 0]"));
}

#[test]
fn emit_model_rejects_expression_invariants_until_data_is_modeled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("workflow.armature");
    std::fs::write(
        &file,
        r#"
machine InvariantWorkflow
initial done

data {
  count int = 0
}

state done {
  final
}

invariant countWithinBound {
  assert data.count <= 3
}
"#,
    )
    .expect("workflow writes");

    let output = armature()
        .arg("emit-model")
        .arg(&file)
        .arg("--target")
        .arg("tla")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("expression invariant `countWithinBound` cannot be represented"));
}

#[test]
fn emit_config_outputs_tla_check_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);

    let output = armature()
        .arg("emit-config")
        .arg(file)
        .arg("--target")
        .arg("tla")
        .output()
        .expect("command runs");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SPECIFICATION Spec"));
    assert!(stdout.contains("INVARIANT DeclaredEffectType"));
    assert!(stdout.contains("INVARIANT CoerceType"));
    assert!(stdout.contains("INVARIANT MaxActiveRespected"));
}

#[test]
fn emit_config_rejects_maude_blank_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);

    let output = armature()
        .arg("emit-config")
        .arg(file)
        .arg("--target")
        .arg("maude")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("separate check config for Maude is not supported"));
}

#[test]
fn emit_config_validates_adapter_manifest_contracts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "empty-adapter",
  "version": "0.1.0",
  "effects": {},
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = armature()
        .arg("emit-config")
        .arg(&file)
        .arg("--target")
        .arg("tla")
        .arg("--adapter-manifest")
        .arg(&manifest)
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("workflow contract validation failed"));
    assert!(stderr.contains("effect `send` is not declared"));
}

#[test]
fn prove_runs_generated_checks_when_formal_tools_are_available() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);

    if !formal_checks_available() {
        eprintln!("skipping prove success test because formal tools are unavailable");
        return;
    }

    let output = armature()
        .arg("prove")
        .arg(file)
        .output()
        .expect("command runs");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("== tla =="));
    assert!(stdout.contains("== maude =="));
    assert!(stdout.contains("ok"));
}

#[test]
fn prove_json_reports_generated_check_results_when_formal_tools_are_available() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);

    if !formal_checks_available() {
        eprintln!("skipping prove json success test because formal tools are unavailable");
        return;
    }

    let output = armature()
        .arg("prove")
        .arg(file)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(output.status.success());
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is json");
    assert_eq!(stdout["ok"], true);
    assert_eq!(stdout["available"], true);
    assert_eq!(
        stdout["suggested_command"],
        "armature check --target tla; armature check --target maude"
    );
    assert_eq!(stdout["checks"].as_array().expect("checks").len(), 2);
    assert_eq!(stdout["checks"][0]["target"], "tla");
    assert_eq!(stdout["checks"][1]["target"], "maude");
}

#[test]
fn prove_validates_adapter_manifest_contracts_before_unavailable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "empty-adapter",
  "version": "0.1.0",
  "effects": {},
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = armature()
        .arg("prove")
        .arg(&file)
        .arg("--adapter-manifest")
        .arg(&manifest)
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("workflow contract validation failed"));
    assert!(stderr.contains("effect `send` is not declared"));
}

#[test]
fn emit_model_outputs_maude_module() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);

    let output = armature()
        .arg("emit-model")
        .arg(file)
        .arg("--target")
        .arg("maude")
        .output()
        .expect("command runs");

    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("mod ARMATURE-CLIWORKFLOW is"));
    assert!(stdout.contains("*** st1 = waiting"));
    assert!(stdout.contains("eq init = st1 ."));
    assert!(stdout.contains("search init =>* S:State"));
}

#[test]
fn emit_model_validates_adapter_manifest_contracts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "empty-adapter",
  "version": "0.1.0",
  "effects": {},
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = armature()
        .arg("emit-model")
        .arg(&file)
        .arg("--target")
        .arg("tla")
        .arg("--adapter-manifest")
        .arg(&manifest)
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("workflow contract validation failed"));
    assert!(stderr.contains("effect `send` is not declared"));
    assert!(stderr.contains("workflow.armature:13:5"));
}

#[test]
fn check_validates_adapter_manifest_contracts_before_running_formal_tools() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_adapter_workflow(&dir);
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "empty-adapter",
  "version": "0.1.0",
  "effects": {},
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = armature()
        .arg("check")
        .arg(&file)
        .arg("--target")
        .arg("tla")
        .arg("--adapter-manifest")
        .arg(&manifest)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("workflow contract validation failed"));
    assert!(stderr.contains("effect `send` is not declared"));
    assert!(stderr.contains("workflow.armature:13:5"));
}

#[test]
fn build_writes_ir_and_tla_artifacts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let out = dir.path().join("build");

    let output = run_json(
        armature()
            .arg("build")
            .arg(&file)
            .arg("--out")
            .arg(&out)
            .arg("--json"),
    );

    let ir_json = output["ir_json"].as_str().expect("ir_json path");
    let baml_src = output["baml_src"].as_str().expect("baml_src path");
    let tla_model = output["tla_model"].as_str().expect("tla_model path");
    let tla_config = output["tla_config"].as_str().expect("tla_config path");
    let maude_model = output["maude_model"].as_str().expect("maude_model path");
    let artifact_hashes_json = output["artifact_hashes_json"]
        .as_str()
        .expect("artifact_hashes_json path");
    assert!(output["adapter_manifest_bundle"].is_null());
    assert!(output["policy_document_bundle"].is_null());
    assert_eq!(output["workflow_id"], "CliWorkflow");
    assert!(std::path::Path::new(ir_json).exists());
    assert!(std::path::Path::new(baml_src).exists());
    assert!(std::path::Path::new(tla_model).exists());
    assert!(std::path::Path::new(tla_config).exists());
    assert!(std::path::Path::new(maude_model).exists());
    assert!(std::path::Path::new(artifact_hashes_json).exists());
    assert!(output["artifact_hashes"]["workflow-ir.json"]
        .as_str()
        .expect("ir hash")
        .starts_with("sha256:"));
    assert!(output["artifact_hashes"]["baml_src/workflow.baml"]
        .as_str()
        .expect("baml hash")
        .starts_with("sha256:"));
    assert!(tla_model.ends_with("Armature_CliWorkflow.tla"));
    assert!(tla_config.ends_with("Armature_CliWorkflow.cfg"));
    assert!(maude_model.ends_with("ARMATURE-CLIWORKFLOW.maude"));
    let artifact_hashes: Value =
        serde_json::from_str(&std::fs::read_to_string(artifact_hashes_json).expect("hashes read"))
            .expect("hashes parse");
    assert_eq!(
        artifact_hashes["workflow-ir.json"],
        output["artifact_hashes"]["workflow-ir.json"]
    );
    let ir = std::fs::read_to_string(ir_json).expect("ir reads");
    assert!(ir.contains("\"schema_version\""));
    assert!(ir.contains(&format!("\"source_path\": \"{}\"", file.display())));
    assert!(std::fs::read_to_string(tla_model)
        .expect("tla reads")
        .contains("---- MODULE Armature_CliWorkflow ----"));
    let tla_config = std::fs::read_to_string(tla_config).expect("tla config reads");
    assert!(tla_config.contains("INVARIANT DeclaredEffectType"));
    assert!(tla_config.contains("INVARIANT CoerceType"));
    assert!(std::fs::read_to_string(maude_model)
        .expect("maude reads")
        .contains("mod ARMATURE-CLIWORKFLOW is"));
}

#[test]
fn build_validates_and_bundles_adapter_manifests() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let out = dir.path().join("build");
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "message-adapter",
  "version": "0.1.0",
  "effects": {
    "send": {
      "category": "message",
      "required_capabilities": ["message_agents"],
      "input": {"type": "json"},
      "output": {"type": "json"},
      "idempotent": true,
      "failure_categories": [],
      "model": {"kind": "opaque"}
    }
  },
  "events": {}
}"#,
    )
    .expect("manifest writes");

    let output = run_json(
        armature()
            .arg("build")
            .arg(&file)
            .arg("--out")
            .arg(&out)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--json"),
    );

    let bundle = output["adapter_manifest_bundle"]
        .as_str()
        .expect("adapter manifest bundle path");
    assert!(std::path::Path::new(bundle).exists());
    assert!(output["artifact_hashes"]["adapter-manifests.json"]
        .as_str()
        .expect("adapter bundle hash")
        .starts_with("sha256:"));
    assert!(std::fs::read_to_string(bundle)
        .expect("bundle reads")
        .contains("\"message-adapter\""));
}

#[test]
fn build_accepts_and_bundles_file_backed_adapter_flags() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let out = dir.path().join("build");
    let agents = dir.path().join("agents.json");

    let output = run_json(
        armature()
            .arg("build")
            .arg(&file)
            .arg("--out")
            .arg(&out)
            .arg("--fixture-agent-file")
            .arg(&agents)
            .arg("--json"),
    );

    let bundle = output["adapter_manifest_bundle"]
        .as_str()
        .expect("adapter manifest bundle path");
    assert!(std::path::Path::new(bundle).exists());
    assert!(std::fs::read_to_string(bundle)
        .expect("bundle reads")
        .contains("\"json-agent-file\""));
}

#[test]
fn build_validates_and_bundles_policy_documents() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let out = dir.path().join("build");
    let manifest = dir.path().join("adapter.json");
    std::fs::write(
        &manifest,
        r#"{
  "name": "message-adapter",
  "version": "0.1.0",
  "effects": {
    "send": {
      "category": "message",
      "required_capabilities": ["message_agents"],
      "input": {"type": "json"},
      "output": {"type": "json"},
      "idempotent": true,
      "failure_categories": [],
      "model": {"kind": "opaque"}
    }
  },
  "events": {}
}"#,
    )
    .expect("manifest writes");
    let policy = dir.path().join("policy.json");
    std::fs::write(
        &policy,
        r#"{
  "mode": "enterprise",
  "allowed_capabilities": ["message_agents"],
  "denied_capabilities": []
}"#,
    )
    .expect("policy writes");

    let output = run_json(
        armature()
            .arg("build")
            .arg(&file)
            .arg("--out")
            .arg(&out)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );

    let bundle = output["policy_document_bundle"]
        .as_str()
        .expect("policy bundle path");
    assert!(std::path::Path::new(bundle).exists());
    assert!(output["artifact_hashes"]["policy-documents.json"]
        .as_str()
        .expect("policy bundle hash")
        .starts_with("sha256:"));
    assert!(std::fs::read_to_string(bundle)
        .expect("bundle reads")
        .contains("\"allowed_capabilities\""));
}

#[test]
fn build_writes_baml_artifact_for_coerce_declarations() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("workflow.armature");
    std::fs::write(
        &file,
        r#"
machine BamlWorkflow
initial done

enum Action {
  Done
}

class Decision {
  action Action
  reason string
  counts map<string, int>
  mode "auto" | "manual"
  enabled true
}

class WorkflowOnly {
  observedAt time
}

coerce choose(planText string) -> Decision {
  model "gpt-4o-mini"
  prompt """
Choose.

{{ planText }}
  """
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    let out = dir.path().join("build");

    let output = run_json(
        armature()
            .arg("build")
            .arg(&file)
            .arg("--out")
            .arg(&out)
            .arg("--json"),
    );

    let baml_src = output["baml_src"].as_str().expect("baml_src path");
    let baml = std::fs::read_to_string(baml_src).expect("baml reads");
    let ir_json = output["ir_json"].as_str().expect("ir_json path");
    let ir = std::fs::read_to_string(ir_json).expect("ir reads");
    assert!(baml.contains("enum Action"));
    assert!(baml.contains("class Decision"));
    assert!(!baml.contains("class WorkflowOnly"));
    assert!(baml.contains("function Choose(planText: string) -> Decision"));
    assert!(baml.contains("counts map<string, int>"));
    assert!(baml.contains("mode \"auto\" | \"manual\""));
    assert!(baml.contains("enabled true"));
    assert!(baml.contains("model \"gpt-4o-mini\""));
    assert!(baml.contains("{{ planText }}"));
    assert!(baml.contains("  prompt #\"\nChoose.\n\n{{ planText }}\n\"#\n"));
    assert!(!baml.contains("\n  \"#"));
    assert!(ir.contains("\"generated_baml_artifact\""));
    assert!(ir.contains("workflow.baml"));
}
