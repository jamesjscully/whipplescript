use serde_json::Value;
use std::process::Command;

fn whipplescript() -> Command {
    Command::new(env!("CARGO_BIN_EXE_whip"))
}

fn formal_checks_available() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(
            "(command -v tlc >/dev/null 2>&1 && command -v maude >/dev/null 2>&1) || command -v nix >/dev/null 2>&1",
        )
        .status()
        .expect("tool probe runs")
        .success()
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
    let file = dir.path().join("workflow.whip");
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
    let file = dir.path().join("workflow.whip");
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

#[test]
fn validate_accepts_workflow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);

    let output = run_json(whipplescript().arg("validate").arg(file).arg("--json"));

    assert_eq!(output["ok"], true);
    assert_eq!(output["diagnostics"], Value::Array(Vec::new()));
}

#[test]
fn init_scaffolds_valid_local_project_and_refuses_overwrite() {
    let dir = tempfile::tempdir().expect("tempdir");

    let output = run_json(
        whipplescript()
            .arg("init")
            .arg(dir.path())
            .arg("--name")
            .arg("DemoWorkflow")
            .arg("--json"),
    );

    let workflow = dir.path().join("workflow.whip");
    let policy = dir.path().join(".whipplescript/policy.json");
    let state_dir = dir.path().join(".whipplescript/state");
    let workflow_store_dir = dir.path().join(".whipplescript/workflows");
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

    let validation = run_json(whipplescript().arg("validate").arg(&workflow).arg("--json"));
    assert_eq!(validation["ok"], true);

    let policy_validation = run_json(whipplescript().arg("validate-policy").arg(&policy).arg("--json"));
    assert_eq!(policy_validation["ok"], true);

    let duplicate = whipplescript()
        .arg("init")
        .arg(dir.path())
        .output()
        .expect("command runs");
    assert!(!duplicate.status.success());
    let stderr = String::from_utf8_lossy(&duplicate.stderr);
    assert!(stderr.contains("refusing to overwrite existing"));

    let forced = whipplescript()
        .arg("init")
        .arg(dir.path())
        .arg("--force")
        .output()
        .expect("command runs");
    assert!(forced.status.success());

    let invalid = whipplescript()
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
    let file = dir.path().join("bad.whip");
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

    let output = whipplescript()
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
    let file = dir.path().join("bad-syntax.whip");
    std::fs::write(
        &file,
        r#"machine
initial waiting
"#,
    )
    .expect("workflow writes");

    let output = whipplescript()
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
    let file = dir.path().join("bad-validation.whip");
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

    let output = whipplescript()
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
    let output = whipplescript()
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
        let output = whipplescript()
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

    let run_help = whipplescript()
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
        whipplescript()
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
        whipplescript()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(events["events"][0]["status"], "queued");

    let queued_events = run_json(
        whipplescript()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--status")
            .arg("queued")
            .arg("--json"),
    );
    assert_eq!(queued_events["events"][0]["event_type"], "go");

    let queued_events_text = whipplescript()
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
        whipplescript()
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
        whipplescript()
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
        whipplescript()
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
        whipplescript()
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
        whipplescript()
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
        whipplescript()
            .arg("status")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--agent-file")
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

    let status_text = whipplescript()
        .arg("status")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .arg("--agent-file")
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
        whipplescript()
            .arg("overview")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--agent-file")
            .arg(&agent_file)
            .arg("--json"),
    );
    assert_eq!(overview["validation"]["ok"], true);
    assert_eq!(overview["status"]["current_state"], "done");
    assert_eq!(overview["status"]["pending_events"], 0);
    assert_eq!(overview["status"]["recent_effects"][0]["effect"], "send");
    assert_eq!(overview["status"]["data_summary"], serde_json::json!({}));
    assert_eq!(overview["status"]["policy_blockers"], serde_json::json!([]));

    let overview_text = whipplescript()
        .arg("overview")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .arg("--agent-file")
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
        whipplescript()
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

    let missing_retry = whipplescript()
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
            whipplescript()
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

        let events_text = whipplescript()
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
            whipplescript()
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
        whipplescript()
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

    let retry_text = whipplescript()
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

    let retry_again = whipplescript()
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
    let file = dir.path().join("data-summary.whip");
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
        whipplescript()
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
        whipplescript()
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
        whipplescript()
            .arg("overview")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(overview["status"]["data_summary"], status["data_summary"]);

    let status_text = whipplescript()
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

    let compact_status = whipplescript()
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
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("baml-http.whip");
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
        whipplescript()
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
fn run_rejects_duplicate_fake_outputs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("fake-duplicates.whip");
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

    let output = whipplescript()
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
    let file = dir.path().join("fake-whitespace.whip");
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

    let output = whipplescript()
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
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("baml-http.whip");
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
        whipplescript()
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
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("baml-http-failure.whip");
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

    let run = whipplescript()
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
        whipplescript()
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

    let status_text = whipplescript()
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
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("coerce-retry.whip");
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

    let failed = whipplescript()
        .arg("run")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .arg("--event")
        .arg("go")
        .arg("--payload")
        .arg(r#"{"message":"hello"}"#)
        .arg("--json")
        .output()
        .expect("command runs");
    assert!(!failed.status.success());

    let failed_events = run_json(
        whipplescript()
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
        whipplescript()
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
        whipplescript()
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

    let status_text = whipplescript()
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
    let file = dir.path().join("baml-http.whip");
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

    let output = whipplescript()
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
    let file = dir.path().join("invalid.whip");
    std::fs::write(
        &file,
        r#"machine
initial waiting
"#,
    )
    .expect("workflow writes");

    let overview = run_json(whipplescript().arg("overview").arg(&file).arg("--json"));

    assert_eq!(overview["validation"]["ok"], false);
    assert!(overview["status"].is_null());
    assert!(overview["validation"]["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|diagnostic| diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("expected machine name"))));

    let output = whipplescript()
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
        whipplescript()
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

    let overview_text = whipplescript()
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

    let output = whipplescript()
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
        whipplescript()
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
        whipplescript()
            .arg("events")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--agent-file")
            .arg(&agent_file)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(events["events"][0]["event_type"], "go");

    let log = run_json(
        whipplescript()
            .arg("log")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--agent-file")
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
        let output = whipplescript()
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
        let output = whipplescript()
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

    let status = whipplescript()
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
        let output = whipplescript()
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
        whipplescript()
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
    let file = dir.path().join("plan-only.whip");
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
        whipplescript()
            .arg("status")
            .arg(&file)
            .arg("--plan-file")
            .arg(&missing_plan)
            .arg("--json"),
    );
    assert_eq!(status["current_state"], "waiting");
    assert_eq!(status["data"]["snapshot"], Value::Null);

    let overview = run_json(
        whipplescript()
            .arg("overview")
            .arg(&file)
            .arg("--plan-file")
            .arg(&missing_plan)
            .arg("--json"),
    );
    assert_eq!(overview["validation"]["ok"], true);
    assert_eq!(overview["status"]["current_state"], "waiting");

    let events = run_json(
        whipplescript()
            .arg("events")
            .arg(&file)
            .arg("--plan-file")
            .arg(&missing_plan)
            .arg("--json"),
    );
    assert_eq!(events["workflow_id"], "PlanOnly");
    assert_eq!(events["events"], Value::Array(Vec::new()));

    let log = run_json(
        whipplescript()
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

    let output = whipplescript()
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
    assert!(stderr.contains("workflow.whip:13:5"));
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

    let output = whipplescript()
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
        .ends_with("workflow.whip"));
    assert_eq!(effect_diagnostic["span"]["start_line"], 13);
    assert_eq!(effect_diagnostic["span"]["start_column"], 5);
}

#[test]
fn validate_accepts_file_backed_adapter_flags_for_static_effect_checks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let agents = dir.path().join("agents.json");

    let output = run_json(
        whipplescript()
            .arg("validate")
            .arg(&file)
            .arg("--agent-file")
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

    let output = whipplescript()
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
        whipplescript()
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

    let output = whipplescript()
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

    let output = whipplescript()
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
        whipplescript()
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

    let output = whipplescript()
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

    let output = whipplescript()
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
    let output = whipplescript()
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

    let output = run_json(whipplescript().arg("validate-policy").arg(&policy).arg("--json"));

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
    let workflow = repo_root.join("examples/templates/simple-agent-supervisor.whip");
    let policy = repo_root.join("examples/policies/local-file-backed.policy.json");
    let agents = repo_root.join("target/tmp/agents.json");

    let output = run_json(
        whipplescript()
            .arg("validate")
            .arg(&workflow)
            .arg("--agent-file")
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

    let output = whipplescript()
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
    let output = whipplescript()
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
        whipplescript()
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

    let overview_text = whipplescript()
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
        whipplescript()
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
    let workflow = repo_root.join("examples/workflows/spec-implementation.whip");
    let manifest = repo_root.join("examples/adapters/spec-implementation.fake-adapter.json");

    let output = run_json(
        whipplescript()
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
    let workflow = repo_root.join("examples/workflows/simple-supervisor.whip");
    let manifest = repo_root.join("examples/adapters/spec-implementation.fake-adapter.json");

    let output = run_json(
        whipplescript()
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

    let output = whipplescript()
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
        whipplescript()
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

    let output = whipplescript()
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

    let output = whipplescript()
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

    let output = whipplescript()
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
    assert!(stdout.contains("---- MODULE WhippleScript_CliWorkflow ----"));
    assert!(stdout.contains(r#"state = "waiting""#));
    assert!(stdout.contains("active = [agent \\in AgentsWithMax |-> 0]"));
}

#[test]
fn emit_model_rejects_expression_invariants_until_data_is_modeled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("workflow.whip");
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

    let output = whipplescript()
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

    let output = whipplescript()
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

    let output = whipplescript()
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

    let output = whipplescript()
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

    let output = whipplescript()
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

    let output = whipplescript()
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
        "whip check --target tla; whip check --target maude"
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

    let output = whipplescript()
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

    let output = whipplescript()
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
    assert!(stdout.contains("mod WHIPPLESCRIPT-CLIWORKFLOW is"));
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

    let output = whipplescript()
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
    assert!(stderr.contains("workflow.whip:13:5"));
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

    let output = whipplescript()
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
    assert!(stderr.contains("workflow.whip:13:5"));
}

#[test]
fn build_writes_ir_and_tla_artifacts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let out = dir.path().join("build");

    let output = run_json(
        whipplescript()
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
    assert!(tla_model.ends_with("WhippleScript_CliWorkflow.tla"));
    assert!(tla_config.ends_with("WhippleScript_CliWorkflow.cfg"));
    assert!(maude_model.ends_with("WHIPPLESCRIPT-CLIWORKFLOW.maude"));
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
        .contains("---- MODULE WhippleScript_CliWorkflow ----"));
    let tla_config = std::fs::read_to_string(tla_config).expect("tla config reads");
    assert!(tla_config.contains("INVARIANT DeclaredEffectType"));
    assert!(tla_config.contains("INVARIANT CoerceType"));
    assert!(std::fs::read_to_string(maude_model)
        .expect("maude reads")
        .contains("mod WHIPPLESCRIPT-CLIWORKFLOW is"));
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
        whipplescript()
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
        whipplescript()
            .arg("build")
            .arg(&file)
            .arg("--out")
            .arg(&out)
            .arg("--agent-file")
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
        whipplescript()
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
    let file = dir.path().join("workflow.whip");
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
        whipplescript()
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
    assert!(baml.contains("function choose(planText: string) -> Decision"));
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
