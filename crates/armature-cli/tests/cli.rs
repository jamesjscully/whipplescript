use serde_json::Value;
use std::process::Command;

fn armature() -> Command {
    Command::new(env!("CARGO_BIN_EXE_armature"))
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

    let output = run_json(armature().arg("validate").arg(file).arg("--json"));

    assert_eq!(output["ok"], true);
    assert_eq!(output["diagnostics"], Value::Array(Vec::new()));
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
fn emit_run_status_events_and_log_share_the_same_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
    let store = dir.path().join("workflow.sqlite");

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

    let status = run_json(
        armature()
            .arg("status")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(status["current_state"], "done");
    assert_eq!(status["recent_effects"][0]["effect"], "send");
    assert_eq!(status["recent_effects"][0]["status"], "succeeded");
    assert_eq!(status["recent_effects"][0]["args"]["message"], "hello");

    let overview = run_json(
        armature()
            .arg("overview")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(overview["validation"]["ok"], true);
    assert_eq!(overview["status"]["current_state"], "done");
    assert_eq!(overview["status"]["pending_events"], 0);
    assert_eq!(overview["status"]["recent_effects"][0]["effect"], "send");

    let overview_text = armature()
        .arg("overview")
        .arg(&file)
        .arg("--store")
        .arg(&store)
        .output()
        .expect("command runs");
    assert!(overview_text.status.success());
    let stdout = String::from_utf8_lossy(&overview_text.stdout);
    assert!(stdout.contains("validation: ok"));
    assert!(stdout.contains("workflow: CliWorkflow"));
    assert!(stdout.contains("state: done"));
    assert!(stdout.contains("latest effects:"));

    let log = run_json(
        armature()
            .arg("log")
            .arg(&file)
            .arg("--store")
            .arg(&store)
            .arg("--json"),
    );
    assert_eq!(log["records"][0]["type"], "effect");
    assert_eq!(log["records"][0]["status"], "succeeded");
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
    let file = write_workflow(&dir);
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
}

#[test]
fn run_rejects_manifest_missing_effect_before_dispatch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
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
    let file = write_workflow(&dir);
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
fn validate_reports_adapter_manifest_input_shape_mismatches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
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
    let file = write_workflow(&dir);
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
}

#[test]
fn validate_applies_enterprise_policy_as_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
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
        .any(|diagnostic| diagnostic["message"]
            .as_str()
            .is_some_and(|message| message.contains("requires capability `message_agents`"))));
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
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("payload does not match schema for event `go`"));
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
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("payload does not match schema for event `go`"));
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
    let file = write_workflow(&dir);
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
fn prove_validates_then_reports_unavailable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);

    let output = armature()
        .arg("prove")
        .arg(file)
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("prove is not implemented yet"));
}

#[test]
fn prove_json_reports_unavailable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);

    let output = armature()
        .arg("prove")
        .arg(file)
        .arg("--json")
        .output()
        .expect("command runs");

    assert!(!output.status.success());
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).expect("stdout is json");
    assert_eq!(stdout["ok"], false);
    assert_eq!(stdout["available"], false);
    assert_eq!(stdout["suggested_command"], "armature check --target tla");
    assert!(String::from_utf8_lossy(&output.stderr).contains("prove is not implemented yet"));
}

#[test]
fn prove_validates_adapter_manifest_contracts_before_unavailable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = write_workflow(&dir);
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
    assert!(!stderr.contains("prove is not implemented yet"));
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
    let file = write_workflow(&dir);
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
    let file = write_workflow(&dir);
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
    assert!(output["adapter_manifest_bundle"].is_null());
    assert!(output["policy_document_bundle"].is_null());
    assert_eq!(output["workflow_id"], "CliWorkflow");
    assert!(std::path::Path::new(ir_json).exists());
    assert!(std::path::Path::new(baml_src).exists());
    assert!(std::path::Path::new(tla_model).exists());
    assert!(std::path::Path::new(tla_config).exists());
    assert!(std::path::Path::new(maude_model).exists());
    assert!(tla_model.ends_with("Armature_CliWorkflow.tla"));
    assert!(tla_config.ends_with("Armature_CliWorkflow.cfg"));
    assert!(maude_model.ends_with("ARMATURE-CLIWORKFLOW.maude"));
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
    assert!(std::fs::read_to_string(bundle)
        .expect("bundle reads")
        .contains("\"message-adapter\""));
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
    assert!(baml.contains("function choose(planText: string) -> Decision"));
    assert!(baml.contains("counts map<string, int>"));
    assert!(baml.contains("mode \"auto\" | \"manual\""));
    assert!(baml.contains("enabled true"));
    assert!(baml.contains("model \"gpt-4o-mini\""));
    assert!(baml.contains("{{ planText }}"));
    assert!(ir.contains("\"generated_baml_artifact\""));
    assert!(ir.contains("workflow.baml"));
}
