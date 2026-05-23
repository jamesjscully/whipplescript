use serde_json::{json, Value};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn armature() -> Command {
    Command::new(env!("CARGO_BIN_EXE_armature"))
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

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives under <repo>/crates/armature-cli")
        .to_path_buf()
}

fn fixture(path: &str) -> PathBuf {
    repo_root().join(path)
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

fn spec_command() -> (PathBuf, PathBuf, PathBuf) {
    (
        fixture("examples/workflows/spec-implementation.armature"),
        fixture("examples/adapters/spec-implementation.fake-adapter.json"),
        fixture("examples/policies/spec-implementation.enterprise-policy.json"),
    )
}

fn active_count(status: &Value, agent: &str) -> u64 {
    status["active_invocations"]
        .as_array()
        .expect("active invocations array")
        .iter()
        .find(|entry| entry["agent"] == agent)
        .and_then(|entry| entry["count"].as_u64())
        .unwrap_or_default()
}

#[test]
fn e2e_spec_implementation_runs_idle_worker_quality_loop_to_done() {
    let (workflow, manifest, policy) = spec_command();
    let dir = tempfile::tempdir().expect("tempdir");
    let store = dir.path().join("spec.sqlite");
    let build_out = dir.path().join("build");

    let validation = run_json(
        armature()
            .arg("validate")
            .arg(&workflow)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(validation["ok"], true);

    let build = run_json(
        armature()
            .arg("build")
            .arg(&workflow)
            .arg("--out")
            .arg(&build_out)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(build["workflow_id"], "specImplementation");
    for key in [
        "ir_json",
        "baml_src",
        "tla_model",
        "tla_config",
        "maude_model",
    ] {
        let path = build[key].as_str().expect("artifact path");
        assert!(Path::new(path).exists(), "{path} should exist");
    }
    let baml_src =
        std::fs::read_to_string(build["baml_src"].as_str().unwrap()).expect("generated baml reads");
    assert!(baml_src.contains("function classifyRun"));
    assert!(baml_src.contains("function chooseNextStep"));

    let idle = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--event")
            .arg("idle")
            .arg("--payload")
            .arg(r#"{"activeRuns":0,"unfinishedItems":2}"#)
            .arg("--fake-call-output")
            .arg(r#"plan.snapshot="W1 is ready""#)
            .arg("--fake-coerce-output")
            .arg(r#"chooseNextStep={"action":"StartWorker","workItemId":"W1","reason":"ready","message":"Implement W1"}"#)
            .arg("--json"),
    );
    assert_eq!(idle["outcome"]["status"], "processed");
    assert_eq!(idle["status"]["current_state"], "watching");
    assert_eq!(active_count(&idle["status"], "worker"), 1);
    assert_eq!(idle["status"]["recent_effects"][0]["effect"], "start");
    assert_eq!(idle["status"]["recent_effects"][0]["status"], "dispatched");

    let worker_done = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"id":"run-worker-1","name":"worker-1","status":"succeeded","stdoutTail":"done","stderrTail":"","exitCode":0}"#)
            .arg("--fake-coerce-output")
            .arg(r#"classifyRun={"kind":"WorkerComplete","workItemId":"W1","reason":"implementation complete"}"#)
            .arg("--json"),
    );
    assert_eq!(worker_done["outcome"]["status"], "processed");
    assert_eq!(worker_done["status"]["current_state"], "watching");
    assert_eq!(active_count(&worker_done["status"], "worker"), 0);
    assert_eq!(active_count(&worker_done["status"], "quality"), 1);
    assert_eq!(
        worker_done["status"]["recent_effects"][0]["target"],
        "quality"
    );

    let duplicate = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"id":"run-worker-1","name":"worker-1","status":"succeeded","stdoutTail":"done","stderrTail":"","exitCode":0}"#)
            .arg("--json"),
    );
    assert_eq!(duplicate["outcome"]["status"], "ignored");
    assert_eq!(active_count(&duplicate["status"], "quality"), 1);

    let quality_done = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"id":"run-quality-1","name":"quality-1","status":"succeeded","stdoutTail":"accepted","stderrTail":"","exitCode":0}"#)
            .arg("--fake-call-output")
            .arg(r#"plan.snapshot="all done""#)
            .arg("--fake-coerce-output")
            .arg(r#"classifyRun={"kind":"QualityPassed","workItemId":"W1","reason":"accepted"}"#)
            .arg("--fake-coerce-output")
            .arg(r#"chooseNextStep={"action":"Done","workItemId":null,"reason":"complete","message":null}"#)
            .arg("--json"),
    );
    assert_eq!(quality_done["outcome"]["status"], "processed");
    assert_eq!(quality_done["status"]["current_state"], "done");
    assert_eq!(
        quality_done["status"]["active_invocations"]
            .as_array()
            .expect("active invocations")
            .len(),
        0
    );

    let overview = run_json(
        armature()
            .arg("overview")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--json"),
    );
    assert_eq!(overview["validation"]["ok"], true);
    assert_eq!(overview["status"]["current_state"], "done");

    let events = run_json(
        armature()
            .arg("events")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    let statuses = events["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|event| event["status"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(statuses.contains(&"processed"));
    assert!(statuses.contains(&"ignored"));
}

#[test]
fn e2e_runtime_recovers_processing_event_from_disk() {
    let workflow = fixture("examples/workflows/simple-supervisor.armature");
    let dir = tempfile::tempdir().expect("tempdir");
    let store = dir.path().join("recovery.sqlite");

    run_json(
        armature()
            .arg("emit")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"name":"worker-1","runId":"run-1","ok":true}"#)
            .arg("--json"),
    );

    let connection = rusqlite::Connection::open(&store).expect("store opens");
    let event_json: String = connection
        .query_row(
            "SELECT event_json FROM workflow_events WHERE status = 'queued'",
            [],
            |row| row.get(0),
        )
        .expect("queued event reads");
    let mut event: Value = serde_json::from_str(&event_json).expect("event json parses");
    event["status"] = json!("processing");
    event["attempt_count"] = json!(1);
    connection
        .execute(
            "UPDATE workflow_events SET status = 'processing', event_json = ?1",
            [serde_json::to_string(&event).expect("event serializes")],
        )
        .expect("event is stranded as processing");
    drop(connection);

    let run = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(run["outcome"]["status"], "processed");
    assert_eq!(run["status"]["current_state"], "watching");
    assert_eq!(run["status"]["pending_events"], 0);

    let events = run_json(
        armature()
            .arg("events")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(events["events"][0]["status"], "processed");
    assert_eq!(events["events"][0]["attempt_count"], 2);
}

#[test]
fn e2e_run_can_use_json_plan_file_adapter() {
    let (workflow, manifest, policy) = spec_command();
    let dir = tempfile::tempdir().expect("tempdir");
    let store = dir.path().join("plan-adapter.sqlite");
    let plan = dir.path().join("plan.json");
    std::fs::write(
        &plan,
        serde_json::to_string_pretty(&json!({
            "tasks": [
                {"id": "W1", "status": "todo", "title": "Implement W1"}
            ]
        }))
        .expect("plan serializes"),
    )
    .expect("plan writes");

    let idle = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--event")
            .arg("idle")
            .arg("--payload")
            .arg(r#"{"activeRuns":0,"unfinishedItems":1}"#)
            .arg("--fake-coerce-output")
            .arg(r#"chooseNextStep={"action":"StartWorker","workItemId":"W1","reason":"ready","message":"Implement W1"}"#)
            .arg("--json"),
    );
    assert_eq!(idle["outcome"]["status"], "processed");
    assert_eq!(active_count(&idle["status"], "worker"), 1);

    let worker_done = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"id":"run-worker-1","name":"worker-1","status":"succeeded","stdoutTail":"done","stderrTail":"","exitCode":0}"#)
            .arg("--fake-coerce-output")
            .arg(r#"classifyRun={"kind":"WorkerComplete","workItemId":"W1","reason":"implementation complete"}"#)
            .arg("--json"),
    );
    assert_eq!(worker_done["outcome"]["status"], "processed");
    assert_eq!(active_count(&worker_done["status"], "quality"), 1);

    let updated_plan: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan).expect("updated plan reads"))
            .expect("updated plan parses");
    assert_eq!(updated_plan["tasks"][0]["status"], "ready_for_quality");
}

#[test]
fn e2e_plan_file_supplies_builtin_plan_manifest_for_plan_only_workflow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("plan-only.armature");
    let store = dir.path().join("plan-only.sqlite");
    let plan = dir.path().join("plan.json");
    std::fs::write(
        &workflow,
        r#"
machine PlanOnly
initial waiting

data {
  snapshot string? = nil
  unfinished int = 0
}

capability plan = adapter("implementationPlan")

event go {}

state waiting {
  on go {
    let text = plan.snapshot()
    let count = plan.unfinishedItems()
    assign data.snapshot = text
    assign data.unfinished = count
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    std::fs::write(
        &plan,
        r#"{"tasks":[{"id":"W1","status":"todo"},{"id":"W0","status":"done"}]}"#,
    )
    .expect("plan writes");

    let run = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--event")
            .arg("go")
            .arg("--json"),
    );

    assert_eq!(run["outcome"]["status"], "processed");
    assert_eq!(run["status"]["current_state"], "done");
    assert!(run["status"]["data"]["snapshot"]
        .as_str()
        .expect("snapshot")
        .contains("\"W1\""));
    assert_eq!(run["status"]["data"]["unfinished"], 1);
}

#[test]
fn e2e_review_file_supplies_builtin_human_review_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("review-only.armature");
    let store = dir.path().join("review-only.sqlite");
    let reviews = dir.path().join("reviews.json");
    std::fs::write(
        &workflow,
        r#"
machine ReviewOnly
initial waiting

event go {
  reason string
}

state waiting {
  on go as evt {
    askHuman(evt.reason)
    goto waiting
  }
}
"#,
    )
    .expect("workflow writes");

    let run = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"reason":"needs human decision"}"#)
            .arg("--json"),
    );

    assert_eq!(run["outcome"]["status"], "processed");
    assert_eq!(run["status"]["recent_effects"][0]["effect"], "askHuman");
    assert_eq!(run["status"]["recent_effects"][0]["status"], "dispatched");

    let reviews: Value =
        serde_json::from_str(&std::fs::read_to_string(&reviews).expect("review file reads"))
            .expect("review file parses");
    assert_eq!(reviews["reviews"][0]["status"], "open");
    assert_eq!(reviews["reviews"][0]["reason"], "needs human decision");
}

#[test]
fn e2e_review_file_supplies_builtin_human_response_event_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("review-response.armature");
    let store = dir.path().join("review-response.sqlite");
    let reviews = dir.path().join("reviews.json");
    std::fs::write(
        &workflow,
        r#"
machine ReviewResponse
initial waiting

state waiting {
  final
}
"#,
    )
    .expect("workflow writes");

    let emitted = run_json(
        armature()
            .arg("emit")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--event")
            .arg("humanReview.responded")
            .arg("--payload")
            .arg(r#"{"reviewId":"review-1","decision":"approved","response":"ship it"}"#)
            .arg("--json"),
    );

    assert_eq!(emitted["event"]["event_type"], "humanReview.responded");
    assert_eq!(emitted["event"]["payload"]["reviewId"], "review-1");
    assert_eq!(emitted["status"]["pending_events"], 1);
    let recorded_response: Value =
        serde_json::from_str(&std::fs::read_to_string(&reviews).expect("review file reads"))
            .expect("review file parses");
    assert_eq!(recorded_response["responses"][0]["reviewId"], "review-1");
    assert_eq!(recorded_response["responses"][0]["decision"], "approved");

    let invalid = armature()
        .arg("emit")
        .arg(&workflow)
        .arg("--store")
        .arg(&store)
        .arg("--review-file")
        .arg(&reviews)
        .arg("--event")
        .arg("humanReview.responded")
        .arg("--payload")
        .arg(r#"{"reviewId":"review-2"}"#)
        .arg("--json")
        .output()
        .expect("command runs");
    assert!(!invalid.status.success());
}

#[test]
fn e2e_agent_file_supplies_builtin_agent_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("agent-only.armature");
    let store = dir.path().join("agent-only.sqlite");
    let agents = dir.path().join("agents.json");
    std::fs::write(
        &workflow,
        r#"
machine AgentOnly
initial waiting

agent director = thread("director")
agent worker = codingAgent()

event go {
  message string
}

state waiting {
  on go as evt {
    start worker {
      task "W1"
      message evt.message
    }
    send director evt.message
    goto waiting
  }
}
"#,
    )
    .expect("workflow writes");

    let run = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--agent-file")
            .arg(&agents)
            .arg("--event")
            .arg("go")
            .arg("--payload")
            .arg(r#"{"message":"please inspect W1"}"#)
            .arg("--json"),
    );

    assert_eq!(run["outcome"]["status"], "processed");
    let recent_effects = run["status"]["recent_effects"]
        .as_array()
        .expect("recent effects array");
    assert!(recent_effects
        .iter()
        .any(|effect| effect["effect"] == "start" && effect["status"] == "dispatched"));
    assert!(recent_effects
        .iter()
        .any(|effect| effect["effect"] == "send" && effect["status"] == "dispatched"));

    let agents: Value =
        serde_json::from_str(&std::fs::read_to_string(&agents).expect("agent file reads"))
            .expect("agent file parses");
    assert_eq!(agents["invocations"][0]["status"], "started");
    assert_eq!(agents["invocations"][0]["agent"], "worker");
    assert_eq!(
        agents["invocations"][0]["input"]["message"],
        "please inspect W1"
    );
    assert_eq!(agents["messages"][0]["status"], "sent");
    assert_eq!(agents["messages"][0]["agent"], "director");
    assert_eq!(agents["messages"][0]["message"], "please inspect W1");
}

#[test]
fn e2e_agent_file_supplies_builtin_finished_event_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("agent-finished.armature");
    let store = dir.path().join("agent-finished.sqlite");
    let agents = dir.path().join("agents.json");
    std::fs::write(
        &workflow,
        r#"
machine AgentFinished
initial waiting

agent worker = codingAgent()

event go {}

state waiting {
  on go {
    start worker
    stay
  }
}
"#,
    )
    .expect("workflow writes");

    let started = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--agent-file")
            .arg(&agents)
            .arg("--event")
            .arg("go")
            .arg("--json"),
    );
    assert_eq!(started["outcome"]["status"], "processed");

    let emitted = run_json(
        armature()
            .arg("emit")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--agent-file")
            .arg(&agents)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(
                r#"{"id":"run-1","name":"worker-1","status":"succeeded","stdoutTail":"","stderrTail":"","exitCode":0}"#,
            )
            .arg("--json"),
    );

    assert_eq!(emitted["event"]["event_type"], "finished");
    assert_eq!(emitted["event"]["payload"]["name"], "worker-1");
    assert_eq!(emitted["status"]["pending_events"], 1);
    let completion_records: Value =
        serde_json::from_str(&std::fs::read_to_string(&agents).expect("agent file reads"))
            .expect("agent file parses");
    assert_eq!(completion_records["invocations"][0]["status"], "finished");
    assert_eq!(
        completion_records["invocations"][0]["completion_id"],
        "run-1"
    );
    assert_eq!(completion_records["completions"][0]["id"], "run-1");
    assert_eq!(completion_records["completions"][0]["name"], "worker-1");

    let invalid = armature()
        .arg("emit")
        .arg(&workflow)
        .arg("--store")
        .arg(&store)
        .arg("--agent-file")
        .arg(&agents)
        .arg("--event")
        .arg("finished")
        .arg("--payload")
        .arg(r#"{"id":"run-2","name":"worker-2"}"#)
        .arg("--json")
        .output()
        .expect("command runs");
    assert!(!invalid.status.success());
}

#[test]
fn e2e_file_backed_adapters_cover_agent_plan_and_review_loop() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workflow = dir.path().join("adapter-loop.armature");
    let store = dir.path().join("adapter-loop.sqlite");
    let plan = dir.path().join("plan.json");
    let reviews = dir.path().join("reviews.json");
    let agents = dir.path().join("agents.json");
    std::fs::write(
        &workflow,
        r#"
machine AdapterLoop
initial watching

agent director = thread("director")
agent worker = codingAgent() {
  maxActive 1
}

capability plan = adapter("implementationPlan")

event idle {}

event finished {
  name string
}

state watching {
  on idle {
    let snapshot = plan.snapshot()
    start worker {
      task "W1"
      message snapshot
    }
    askHuman("worker started")
    goto watching
  }

  on finished as run {
    plan.markDone("W1")
    send director run.name
    goto done
  }
}

state done {
  final
}
"#,
    )
    .expect("workflow writes");
    std::fs::write(&plan, r#"{"tasks":[{"id":"W1","status":"todo"}]}"#).expect("plan writes");

    let first = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--agent-file")
            .arg(&agents)
            .arg("--event")
            .arg("idle")
            .arg("--json"),
    );
    assert_eq!(first["outcome"]["status"], "processed");
    assert_eq!(first["status"]["current_state"], "watching");
    assert_eq!(active_count(&first["status"], "worker"), 1);

    let agent_records: Value =
        serde_json::from_str(&std::fs::read_to_string(&agents).expect("agent file reads"))
            .expect("agent file parses");
    assert_eq!(agent_records["invocations"][0]["agent"], "worker");
    assert_eq!(agent_records["invocations"][0]["input"]["task"], "W1");

    let review_records: Value =
        serde_json::from_str(&std::fs::read_to_string(&reviews).expect("review file reads"))
            .expect("review file parses");
    assert_eq!(review_records["reviews"][0]["status"], "open");
    assert_eq!(review_records["reviews"][0]["reason"], "worker started");

    let review_id = review_records["reviews"][0]["id"]
        .as_str()
        .expect("review id")
        .to_string();

    let review_response = run_json(
        armature()
            .arg("emit")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--event")
            .arg("humanReview.responded")
            .arg("--payload")
            .arg(format!(
                r#"{{"reviewId":"{review_id}","decision":"approved","response":"continue"}}"#
            ))
            .arg("--json"),
    );
    assert_eq!(
        review_response["event"]["event_type"],
        "humanReview.responded"
    );
    let responded_reviews: Value =
        serde_json::from_str(&std::fs::read_to_string(&reviews).expect("review file reads"))
            .expect("review file parses");
    assert_eq!(responded_reviews["reviews"][0]["status"], "responded");
    assert_eq!(responded_reviews["reviews"][0]["decision"], "approved");
    assert_eq!(responded_reviews["reviews"][0]["response"], "continue");

    let ignored_review_response = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--agent-file")
            .arg(&agents)
            .arg("--json"),
    );
    assert_eq!(ignored_review_response["outcome"]["status"], "ignored");

    let second = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--plan-file")
            .arg(&plan)
            .arg("--review-file")
            .arg(&reviews)
            .arg("--agent-file")
            .arg(&agents)
            .arg("--event")
            .arg("finished")
            .arg("--payload")
            .arg(r#"{"name":"worker-01"}"#)
            .arg("--json"),
    );
    assert_eq!(second["outcome"]["status"], "processed");
    assert_eq!(second["status"]["current_state"], "done");
    assert_eq!(active_count(&second["status"], "worker"), 0);

    let updated_plan: Value =
        serde_json::from_str(&std::fs::read_to_string(&plan).expect("plan file reads"))
            .expect("plan file parses");
    assert_eq!(updated_plan["tasks"][0]["status"], "done");

    let updated_agents: Value =
        serde_json::from_str(&std::fs::read_to_string(&agents).expect("agent file reads"))
            .expect("agent file parses");
    assert_eq!(updated_agents["messages"][0]["agent"], "director");
    assert_eq!(updated_agents["messages"][0]["message"], "worker-01");
}

#[test]
fn e2e_formal_check_commands_run_against_real_fixture_when_tools_exist() {
    let (workflow, manifest, policy) = spec_command();

    if !formal_checks_available() {
        eprintln!("skipping formal e2e because formal tools are unavailable");
        return;
    }

    let tla = run_json(
        armature()
            .arg("check")
            .arg(&workflow)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--target")
            .arg("tla")
            .arg("--json"),
    );
    assert_eq!(tla["ok"], true);
    assert_eq!(tla["target"], "tla");

    let maude = run_json(
        armature()
            .arg("check")
            .arg(&workflow)
            .arg("--adapter-manifest")
            .arg(&manifest)
            .arg("--policy")
            .arg(&policy)
            .arg("--target")
            .arg("maude")
            .arg("--json"),
    );
    assert_eq!(maude["ok"], true);
    assert_eq!(maude["target"], "maude");
}

#[test]
fn e2e_real_baml_http_coerce_when_enabled() {
    if env::var("ARMATURE_RUN_BAML_E2E").ok().as_deref() != Some("1") {
        eprintln!("skipping real BAML e2e because ARMATURE_RUN_BAML_E2E=1 is not set");
        return;
    }

    let baml_url =
        env::var("ARMATURE_BAML_URL").expect("ARMATURE_BAML_URL is required for real BAML e2e");
    let workflow = fixture("examples/workflows/baml-coerce-smoke.armature");
    let dir = tempfile::tempdir().expect("tempdir");
    let store = dir.path().join("baml.sqlite");
    let build_out = dir.path().join("build");

    let build = run_json(
        armature()
            .arg("build")
            .arg(&workflow)
            .arg("--out")
            .arg(&build_out)
            .arg("--json"),
    );
    let baml_src =
        std::fs::read_to_string(build["baml_src"].as_str().unwrap()).expect("generated baml reads");
    assert!(baml_src.contains("function classifyText"));

    let run = run_json(
        armature()
            .arg("run")
            .arg(&workflow)
            .arg("--store")
            .arg(&store)
            .arg("--event")
            .arg("classify")
            .arg("--payload")
            .arg(r#"{"text":"The workflow completed successfully."}"#)
            .arg("--baml-url")
            .arg(&baml_url)
            .arg("--json"),
    );

    assert_eq!(run["outcome"]["status"], "processed");
    assert_eq!(run["status"]["current_state"], "done");
    assert_eq!(
        run["status"]["latest_coerce_calls"][0]["function_name"],
        "classifyText"
    );
    assert_eq!(
        run["status"]["latest_coerce_calls"][0]["status"],
        "succeeded"
    );
    assert_eq!(
        run["status"]["latest_coerce_calls"][0]["backend"]["kind"],
        "baml_http"
    );
    assert!(run["status"]["data"]["reason"].as_str().is_some());
}

#[test]
fn e2e_no_default_runtime_state_is_tracked_in_examples() {
    let default_store = fixture("examples/workflows/.armature/workflows/Minimal.sqlite");

    let ignored = Command::new("git")
        .arg("check-ignore")
        .arg(&default_store)
        .current_dir(repo_root())
        .output()
        .expect("git check-ignore runs");
    assert!(
        ignored.status.success(),
        "default workflow store should be ignored"
    );
}
