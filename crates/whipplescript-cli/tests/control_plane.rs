use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};
use whipplescript_store::{
    ArtifactRecord, EffectCompletion, NewFact, RuleCommit, RunStart, SqliteStore,
};

#[test]
fn checks_all_example_workflows() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let examples = [
        "minimal-noop.whip",
        "ralph.whip",
        "queue-worker-with-review.whip",
        "queue-gated-smoke.whip",
        "coerce-branch.whip",
        "terminal-output-union.whip",
        "triage-flow.whip",
        "incident-router.whip",
        "scheduled-escalation.whip",
        "event-bridge.whip",
        "reusable-review-pattern.whip",
        "exec-json-ingest.whip",
        "autoresearch-lite.whip",
        "gastown-lite.whip",
        "circuit-breaker.whip",
        "human-review.whip",
        "multi-agent-bounded-concurrency.whip",
        "openclaw-lite.whip",
        "plugin-memory.whip",
        "provider-language-e2e.whip",
    ];
    let paths = examples
        .iter()
        .map(|name| example_path(name))
        .collect::<Vec<_>>();
    let mut args = vec!["check"];
    let path_strings = paths
        .iter()
        .map(|path| path.to_str().expect("example path is utf-8"))
        .collect::<Vec<_>>();
    args.extend(path_strings);

    let output = run_text(bin, &args);

    for example in examples {
        assert!(output.contains(example), "{output}");
    }
}

#[test]
fn check_json_reports_source_metadata() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let example = example_path("provider-language-e2e.whip");

    let report = run_json(
        bin,
        &[
            "--json",
            "check",
            example.to_str().expect("utf-8 example path"),
        ],
    );
    let reports = report.as_array().expect("check json report array");
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].get("schema").and_then(Value::as_str),
        Some("whipplescript.check_report.v0")
    );
    let source_metadata = reports[0].get("source_metadata").expect("source metadata");
    let targets = source_metadata
        .get("targets")
        .and_then(Value::as_object)
        .expect("metadata targets");

    let workflow = targets
        .get("workflow:ProviderLanguageE2E")
        .expect("workflow metadata");
    assert_eq!(
        workflow.get("description").and_then(Value::as_str),
        Some("Fixture-backed provider x language acceptance workflow")
    );
    assert!(workflow
        .get("tags")
        .and_then(Value::as_array)
        .expect("workflow tags")
        .iter()
        .any(|tag| tag.as_str() == Some("acceptance")));

    let table = targets.get("table:language_tasks").expect("table metadata");
    assert_eq!(
        table.get("description").and_then(Value::as_str),
        Some("Static provider x language task rows")
    );
    assert!(table
        .get("tags")
        .and_then(Value::as_array)
        .expect("table tags")
        .iter()
        .any(|tag| tag.as_str() == Some("fixture")));

    let rule = targets
        .get("rule:run_language_task")
        .expect("rule metadata");
    assert_eq!(
        rule.get("description").and_then(Value::as_str),
        Some("Route one queued language task to its selected provider")
    );
}

#[test]
fn compile_json_reports_source_metadata() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let example = example_path("provider-language-e2e.whip");

    let report = run_json(
        bin,
        &[
            "--json",
            "compile",
            example.to_str().expect("utf-8 example path"),
        ],
    );

    assert_eq!(
        report.get("schema").and_then(Value::as_str),
        Some("whipplescript.compile_report.v0")
    );
    assert_eq!(
        report.get("workflow").and_then(Value::as_str),
        Some("ProviderLanguageE2E")
    );
    assert!(report
        .get("source_metadata")
        .and_then(|metadata| metadata.get("descriptions"))
        .and_then(Value::as_array)
        .expect("descriptions")
        .iter()
        .any(
            |description| description.get("target_kind").and_then(Value::as_str) == Some("rule")
                && description.get("target").and_then(Value::as_str) == Some("run_language_task")
        ));
}

#[test]
fn dev_table_rows_report_fact_provenance_and_source_spans() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("table-row-report");
    fs::write(
        &source_path,
        r#"
workflow TableRowReport

class Task {
  title string
  status "queued"
}

table tasks as Task [
  {
    title "Review parser"
    status "queued"
  }

  {
    title "Review runtime"
    status "queued"
  }
]
"#,
    )
    .expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let task_facts = facts
        .as_array()
        .expect("facts")
        .iter()
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("Task"))
        .collect::<Vec<_>>();
    assert_eq!(task_facts.len(), 2);
    assert!(task_facts.iter().all(|fact| {
        fact.get("provenance_class").and_then(Value::as_str) == Some("table")
            && fact
                .pointer("/source_span/construct")
                .and_then(Value::as_str)
                == Some("table_row")
            && fact.pointer("/source_span/path").and_then(Value::as_str) == source_path.to_str()
            && fact
                .pointer("/source_span/start")
                .and_then(Value::as_u64)
                .is_some()
            && fact
                .pointer("/source_span/end")
                .and_then(Value::as_u64)
                .is_some()
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn human_answer_fires_dependent_rule() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("human-review.whip");
    let store = store_path.to_str().expect("utf-8 temp path");
    let source = example.to_str().expect("utf-8 example path");

    let dev = run_json(
        bin,
        &[
            "--store",
            store,
            "--json",
            "dev",
            source,
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();

    let inbox = run_json(bin, &["--store", store, "--json", "inbox"]);
    let items = inbox.as_array().expect("inbox items");
    assert_eq!(items.len(), 1, "one pending human review item");
    let inbox_item_id = items[0]
        .get("inbox_item_id")
        .and_then(Value::as_str)
        .expect("inbox item id")
        .to_owned();

    run_text(
        bin,
        &[
            "--store",
            store,
            "inbox",
            "answer",
            &inbox_item_id,
            "--choice",
            "accept",
        ],
    );
    run_text(
        bin,
        &["--store", store, "step", &instance_id, "--program", source],
    );

    let facts = run_json(bin, &["--store", store, "--json", "facts", &instance_id]);
    let decisions = facts
        .as_array()
        .expect("facts")
        .iter()
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("HumanDecision"))
        .collect::<Vec<_>>();
    assert_eq!(decisions.len(), 1, "answer fires record_manual_review");
    assert_eq!(
        decisions[0]
            .pointer("/value/decision")
            .and_then(Value::as_str),
        Some("accept")
    );

    let _ = fs::remove_file(store_path);
}

#[test]
fn completed_turn_pattern_fires_dependent_rule() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("completed-turn");
    fs::write(
        &source_path,
        r#"
workflow CompletedTurn

output result TurnSeen

class TurnSeen {
  agent string
  summary string
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule begin
  when started
  when worker is available
=> {
  tell worker "Do the task."
}

rule observe
  when worker completed turn as turn
=> {
  record TurnSeen {
    agent turn.agent
    summary turn.summary
  }

  complete result {
    agent turn.agent
    summary turn.summary
  }
}
"#,
    )
    .expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let seen = facts
        .as_array()
        .expect("facts")
        .iter()
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("TurnSeen"))
        .collect::<Vec<_>>();
    assert_eq!(seen.len(), 1, "completed turn fires observe rule");
    assert_eq!(
        seen[0].pointer("/value/agent").and_then(Value::as_str),
        Some("worker")
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_same_workflow_twice_in_one_store_creates_distinct_instances() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("repeat-run");
    fs::write(
        &source_path,
        r#"
workflow RepeatRun

class Task {
  title string
  status string
}

class Finished {
  title string
  status "finished"
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

table tasks as Task [
  {
    title "Review parser"
    status "queued"
  }
]

rule run_task
  when Task as task where task.status == "queued"
  when worker is available
=> {
  tell worker as turn "Do {{ task.title }}"

  after turn succeeds as completed {
    done task -> record Finished {
      title task.title
      status "finished"
    }
  }
}
"#,
    )
    .expect("write source");

    let mut instance_ids = Vec::new();
    for _ in 0..2 {
        let dev = run_json(
            bin,
            &[
                "--store",
                store_path.to_str().expect("utf-8 temp path"),
                "--json",
                "dev",
                source_path.to_str().expect("utf-8 source path"),
                "--provider",
                "fixture",
                "--until",
                "idle",
            ],
        );
        let instance_id = dev
            .get("instance_id")
            .and_then(Value::as_str)
            .expect("instance id")
            .to_owned();
        instance_ids.push(instance_id);
    }

    assert_ne!(instance_ids[0], instance_ids[1]);
    for instance_id in &instance_ids {
        let facts = run_json(
            bin,
            &[
                "--store",
                store_path.to_str().expect("utf-8 temp path"),
                "--json",
                "facts",
                instance_id,
            ],
        );
        let finished = facts
            .as_array()
            .expect("facts")
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("Finished"))
            .count();
        assert_eq!(finished, 1, "instance {instance_id} completed its task");
    }

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_include_tag_filters_assertions_without_skipping_runtime() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("tag-filter");
    fs::write(
        &source_path,
        r#"
workflow TagFilter

class Seen {
  status "ok"
}

@smoke
description "Selected smoke assertion passes"
assert count(Seen) == 1

@slow
description "Unselected slow assertion would fail"
assert count(Seen) == 2

rule seed
  when started
=> {
  record Seen {
    status "ok"
  }
}
"#,
    )
    .expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
            "--include-tag",
            "smoke",
        ],
    );
    assert_eq!(
        dev.pointer("/assertion_filter/total")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        dev.pointer("/assertion_filter/selected")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        dev.pointer("/executable_spec/status")
            .and_then(Value::as_str),
        Some("passed")
    );
    let assertions = dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions");
    assert_eq!(assertions.len(), 1);
    assert_eq!(
        assertions[0].get("description").and_then(Value::as_str),
        Some("Selected smoke assertion passes")
    );
    assert!(assertions[0]
        .get("tags")
        .and_then(Value::as_array)
        .expect("tags")
        .iter()
        .any(|tag| tag.as_str() == Some("smoke")));

    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    assert_eq!(
        facts
            .as_array()
            .expect("facts")
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("Seen"))
            .count(),
        1
    );

    let exclude_store_path = temp_store_path();
    let exclude_dev = run_json(
        bin,
        &[
            "--store",
            exclude_store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
            "--exclude-tag",
            "slow",
        ],
    );
    assert_eq!(
        exclude_dev
            .pointer("/assertion_filter/selected")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        exclude_dev
            .pointer("/assertions/0/description")
            .and_then(Value::as_str),
        Some("Selected smoke assertion passes")
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(exclude_store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn check_resolves_relative_whip_includes() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let root = temp_workflow_path("include-root");
    let lib = root.with_file_name(format!(
        "{}.lib.whip",
        root.file_stem()
            .expect("temp path has stem")
            .to_string_lossy()
    ));
    fs::write(
        &lib,
        r#"class IncludedTask {
  id string
}
"#,
    )
    .expect("write include lib");
    fs::write(
        &root,
        format!(
            r#"include "{}"

@service
workflow IncludeRoot

rule noop
=> {{
  record IncludedTask {{
    id "task-1"
  }}
}}
"#,
            lib.file_name().expect("lib file name").to_string_lossy()
        ),
    )
    .expect("write root workflow");

    let output = run_text(bin, &["check", root.to_str().expect("utf-8 workflow path")]);
    assert!(output.contains("includes"), "{output}");
    assert!(output.contains("IncludedTask"), "{output}");

    let _ = fs::remove_file(root);
    let _ = fs::remove_file(lib);
}

#[test]
fn doctor_providers_reports_deterministic_health_posture() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();

    let doctor = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "doctor",
            "--providers",
        ],
    );
    let checks = doctor
        .get("provider_health_checks")
        .and_then(Value::as_array)
        .expect("provider health checks");
    for provider in ["codex", "claude", "pi"] {
        assert!(
            checks
                .iter()
                .any(|check| check.get("provider").and_then(Value::as_str) == Some(provider)),
            "missing health check for {provider}: {checks:?}"
        );
    }
    assert!(!checks.iter().any(|check| {
        check.get("provider").and_then(Value::as_str) == Some("fixture")
            || check.get("provider").and_then(Value::as_str) == Some("command")
    }));
    assert!(checks.iter().all(|check| {
        matches!(
            check.get("status").and_then(Value::as_str),
            Some("pass" | "fail" | "skip")
        )
    }));
    let doctor_json = doctor.to_string();
    assert!(!doctor_json.contains("sk-test-secret"), "{doctor_json}");
    assert!(!doctor_json.contains("ANTHROPIC_API_KEY="), "{doctor_json}");
    assert!(!doctor_json.contains("OPENAI_API_KEY="), "{doctor_json}");
    assert!(!doctor_json.contains("PI_API_KEY="), "{doctor_json}");

    let _ = fs::remove_file(store_path);
}

#[test]
fn check_root_option_validates_current_workflow_name() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let source_path = temp_workflow_path("root-selection");
    fs::write(
        &source_path,
        r#"
@service
workflow SelectedRoot

rule noop
  when started
=> {
}
"#,
    )
    .expect("write workflow");

    let ok = run_text(
        bin,
        &[
            "check",
            "--root",
            "SelectedRoot",
            source_path.to_str().expect("utf-8 workflow path"),
        ],
    );
    assert!(ok.contains("workflow SelectedRoot"), "{ok}");

    let failed = Command::new(bin)
        .args([
            "check",
            "--root",
            "MissingRoot",
            source_path.to_str().expect("utf-8 workflow path"),
        ])
        .output()
        .expect("command runs");
    assert!(!failed.status.success());
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(
        stderr.contains("root workflow `MissingRoot` was not found"),
        "{stderr}"
    );
    assert!(
        stderr.contains("available workflow: `SelectedRoot`"),
        "{stderr}"
    );

    let _ = fs::remove_file(source_path);
}

#[test]
fn check_rejects_duplicate_includes_in_one_file() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let root = temp_workflow_path("duplicate-include-root");
    let lib = root.with_file_name(format!(
        "{}.lib.whip",
        root.file_stem()
            .expect("temp path has stem")
            .to_string_lossy()
    ));
    fs::write(
        &lib,
        r#"class Included {
  id string
}
"#,
    )
    .expect("write include lib");
    let include_name = lib.file_name().expect("lib file name").to_string_lossy();
    fs::write(
        &root,
        format!(
            r#"include "{include_name}"
include "{include_name}"

workflow DuplicateInclude
"#
        ),
    )
    .expect("write root workflow");

    let output = Command::new(bin)
        .args(["check", root.to_str().expect("utf-8 workflow path")])
        .output()
        .expect("command runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("duplicate include"), "{stderr}");

    let _ = fs::remove_file(root);
    let _ = fs::remove_file(lib);
}

#[test]
fn check_selects_root_from_multiple_explicit_workflows() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let source_path = temp_workflow_path("multi-root");
    fs::write(
        &source_path,
        r#"
class Selected {
  id string
}

@service
workflow Alpha {
  rule alpha
    when started
  => {
    record Selected {
      id "alpha"
    }
  }
}

@service
workflow Beta {
  rule beta
    when started
  => {
    record Selected {
      id "beta"
    }
  }
}
"#,
    )
    .expect("write workflow");

    let ambiguous = Command::new(bin)
        .args(["check", source_path.to_str().expect("utf-8 workflow path")])
        .output()
        .expect("command runs");
    assert!(!ambiguous.status.success());
    let stderr = String::from_utf8_lossy(&ambiguous.stderr);
    assert!(
        stderr.contains("multiple workflow declarations require an explicit root"),
        "{stderr}"
    );

    let output = run_text(
        bin,
        &[
            "check",
            "--root",
            "Beta",
            source_path.to_str().expect("utf-8 workflow path"),
        ],
    );
    assert!(output.contains("workflow Beta"), "{output}");
    assert!(output.contains("rule beta"), "{output}");
    assert!(!output.contains("rule alpha"), "{output}");

    let _ = fs::remove_file(source_path);
}

#[test]
fn starts_and_inspects_two_instances_independently() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("ralph.whip");

    let first = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            example.to_str().expect("utf-8 example path"),
            "--input",
            r#"{"ticket":"one"}"#,
            "--json",
        ],
    );
    let second = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            example.to_str().expect("utf-8 example path"),
            "--input",
            r#"{"ticket":"two"}"#,
            "--json",
        ],
    );

    let first_id = first
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("first instance id");
    let second_id = second
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("second instance id");
    assert_ne!(first_id, second_id);
    assert_eq!(first.get("program_id"), second.get("program_id"));
    assert_eq!(first.get("version_id"), second.get("version_id"));

    let instances = run_text(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "instances",
        ],
    );
    assert!(instances.contains(first_id), "{instances}");
    assert!(instances.contains(second_id), "{instances}");

    let first_status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "status",
            first_id,
            "--json",
        ],
    );
    let second_status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "status",
            second_id,
            "--json",
        ],
    );
    assert_eq!(ticket(&first_status), Some("one"));
    assert_eq!(ticket(&second_status), Some("two"));
    assert_eq!(
        first_status
            .get("recent_events")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    let first_trace = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "trace",
            first_id,
            "--json",
        ],
    );
    assert_eq!(
        first_trace.get("schema").and_then(Value::as_str),
        Some("whipplescript.local_trace.v0")
    );
    assert_eq!(
        first_trace
            .get("events")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    let checked_trace = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "trace",
            first_id,
            "--check",
            "--json",
        ],
    );
    assert_eq!(
        checked_trace
            .get("conformance")
            .and_then(|value| value.get("ok"))
            .and_then(Value::as_bool),
        Some(true)
    );
    let first_evidence = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "evidence",
            first_id,
            "--json",
        ],
    );
    assert_eq!(
        first_evidence
            .get("evidence")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );

    let _ = fs::remove_file(store_path);
}

#[test]
fn artifacts_command_lists_metadata_without_raw_content() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("artifact-metadata");
    let secret = "sk-test-secret-token-1234567890";
    fs::write(
        &workflow_path,
        r#"
workflow ArtifactMetadata

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule start_work
  when started
  when worker is available
=> {
  tell worker "collect artifact metadata"
}
"#,
    )
    .expect("write workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            workflow_path.to_str().expect("utf-8 workflow path"),
        ],
    );
    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    let effect_id = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("agent.tell"))
        .and_then(|effect| effect.get("effect_id"))
        .and_then(Value::as_str)
        .expect("agent effect id")
        .to_owned();

    let mut store = SqliteStore::open(&store_path).expect("open store");
    store
        .start_run(RunStart {
            instance_id,
            effect_id: &effect_id,
            run_id: "run-artifact-metadata",
            provider: "fixture",
            worker_id: "worker-1",
            lease_id: "lease-artifact-metadata",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("run starts");
    store
        .record_artifact(ArtifactRecord {
            run_id: "run-artifact-metadata",
            kind: "transcript",
            path: &format!("provider://fixture/runs/run-artifact-metadata/{secret}/transcript"),
            content_hash: Some(&format!("sha256:{secret}")),
            mime_type: Some("text/plain"),
        })
        .expect("artifact records");
    drop(store);

    let artifacts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "artifacts",
            "run-artifact-metadata",
        ],
    );
    let artifacts_json = artifacts.to_string();
    assert!(!artifacts_json.contains(secret), "{artifacts_json}");
    assert!(!artifacts_json.contains("content\""), "{artifacts_json}");
    let artifact = artifacts
        .get("artifacts")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .expect("artifact row");
    assert_eq!(
        artifact.get("path").and_then(Value::as_str),
        Some("[REDACTED]")
    );
    assert_eq!(
        artifact.get("content_hash").and_then(Value::as_str),
        Some("[REDACTED]")
    );
    let runs = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "runs",
            instance_id,
        ],
    );
    assert!(runs.as_array().expect("runs").iter().any(|run| {
        run.get("run_id").and_then(Value::as_str) == Some("run-artifact-metadata")
            && run.get("artifact_count").and_then(Value::as_u64) == Some(1)
    }));
    let trace = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "trace",
            instance_id,
        ],
    );
    assert!(trace
        .get("runs")
        .and_then(Value::as_array)
        .expect("trace runs")
        .iter()
        .any(|run| {
            run.get("run_id").and_then(Value::as_str) == Some("run-artifact-metadata")
                && run.get("artifact_count").and_then(Value::as_u64) == Some(1)
        }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn revise_dry_run_reports_compatibility_without_mutating_instance() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("revise-dry-run-v1");
    let v2 = temp_workflow_path("revise-dry-run-v2");
    fs::write(
        &v1,
        r#"
workflow ReviseDemo

rule noop
  when started
=> {
}
"#,
    )
    .expect("write v1 workflow");
    fs::write(
        &v2,
        r#"
workflow ReviseDemo

rule noop_v2
  when started
=> {
}
"#,
    )
    .expect("write v2 workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let original_version = started
        .get("version_id")
        .and_then(Value::as_str)
        .expect("version id")
        .to_owned();

    let report = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--dry-run",
            "--cancel",
            "queued",
            "--json",
        ],
    );
    assert_eq!(report.get("dry_run").and_then(Value::as_bool), Some(true));
    assert_eq!(
        report
            .get("compatibility")
            .and_then(|value| value.get("compatible"))
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        report
            .pointer("/would_create/diagnostics")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        report
            .pointer("/would_create/evidence/0/kind")
            .and_then(Value::as_str),
        Some("workflow.revision.dry_run")
    );
    assert_eq!(
        report
            .pointer("/would_create/evidence/0/metadata/compatible")
            .and_then(Value::as_bool),
        Some(true)
    );

    let status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "status",
            instance_id,
            "--json",
        ],
    );
    assert_eq!(
        status
            .get("instance")
            .and_then(|instance| instance.get("version_id"))
            .and_then(Value::as_str),
        Some(original_version.as_str())
    );
    assert_eq!(
        status
            .get("revisions")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn revise_activation_updates_active_version_and_status_history() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("revise-activate-v1");
    let v2 = temp_workflow_path("revise-activate-v2");
    fs::write(
        &v1,
        r#"
workflow ReviseActivate

rule noop
  when started
=> {
}
"#,
    )
    .expect("write v1 workflow");
    fs::write(
        &v2,
        r#"
workflow ReviseActivate

rule noop_v2
  when started
=> {
}
"#,
    )
    .expect("write v2 workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    let activation = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--cancel",
            "keep",
            "--json",
        ],
    );
    assert_eq!(
        activation.get("dry_run").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        activation
            .get("revision")
            .and_then(|revision| revision.get("epoch"))
            .and_then(Value::as_i64),
        Some(1)
    );

    let status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "status",
            instance_id,
            "--json",
        ],
    );
    assert_eq!(
        status
            .get("instance")
            .and_then(|instance| instance.get("revision_epoch"))
            .and_then(Value::as_i64),
        Some(1)
    );
    assert_eq!(
        status
            .get("revisions")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn step_rejects_stale_program_after_revision() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("step-revision-v1");
    let v2 = temp_workflow_path("step-revision-v2");
    fs::write(
        &v1,
        r#"
workflow StepRevision

class Marker {
  version string
}

rule seed_v1
  when started
=> {
  record Marker {
    version "v1"
  }
}
"#,
    )
    .expect("write v1 workflow");
    fs::write(
        &v2,
        r#"
workflow StepRevision

class Marker {
  version string
}

rule seed_v2
  when started
=> {
  record Marker {
    version "v2"
  }
}
"#,
    )
    .expect("write v2 workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );

    let stale_step = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "step",
            instance_id,
            "--program",
            v1.to_str().expect("utf-8 workflow path"),
        ])
        .output()
        .expect("command runs");
    assert!(!stale_step.status.success());
    let stderr = String::from_utf8_lossy(&stale_step.stderr);
    assert!(stderr.contains("does not match active version"), "{stderr}");
    let diagnostics = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "diagnostics",
            instance_id,
        ],
    );
    let diagnostics = diagnostics.as_array().expect("diagnostics array");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.get("code").and_then(Value::as_str) == Some("revision.stale_program_path")
            && diagnostic.get("subject_type").and_then(Value::as_str) == Some("program_path")
            && diagnostic
                .get("program_version_id")
                .and_then(Value::as_str)
                .is_some()
    }));

    let step = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            v2.to_str().expect("utf-8 workflow path"),
        ],
    );
    assert_eq!(step.get("committed_rules").and_then(Value::as_u64), Some(1));

    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("Marker")
            && fact
                .get("value")
                .and_then(|value| value.get("version"))
                .and_then(Value::as_str)
                == Some("v2")
    }));
    assert!(!facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("Marker")
            && fact
                .get("value")
                .and_then(|value| value.get("version"))
                .and_then(Value::as_str)
                == Some("v1")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn revise_records_missing_source_bundle_diagnostic() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("revise-missing-source-v1");
    let missing = temp_workflow_path("revise-missing-source-v2");
    fs::write(
        &v1,
        r#"
workflow RevisionMissingSource

rule noop
  when started
=> {
}
"#,
    )
    .expect("write v1 workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    let revise = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            missing.to_str().expect("utf-8 workflow path"),
            "--json",
        ])
        .output()
        .expect("command runs");
    assert!(!revise.status.success());
    let stderr = String::from_utf8_lossy(&revise.stderr);
    assert!(stderr.contains("failed to read"), "{stderr}");

    let diagnostics = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "diagnostics",
            instance_id,
        ],
    );
    let diagnostics = diagnostics.as_array().expect("diagnostics array");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.get("code").and_then(Value::as_str) == Some("revision.source_bundle_unavailable")
            && diagnostic.get("subject_type").and_then(Value::as_str)
                == Some("revision_source_bundle")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(missing);
}

#[test]
fn old_effect_runs_after_keep_revision_with_old_attribution() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("old-effect-keep-v1");
    let v2 = temp_workflow_path("old-effect-keep-v2");
    fs::write(
        &v1,
        r#"
workflow OldEffectKeep

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule start_old_work
  when started
  when worker is available
=> {
  tell worker "old work"
}
"#,
    )
    .expect("write v1 workflow");
    fs::write(
        &v2,
        r#"
workflow OldEffectKeep

rule noop_v2
  when started
=> {
}
"#,
    )
    .expect("write v2 workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let original_version = started
        .get("version_id")
        .and_then(Value::as_str)
        .expect("version id");

    let first_step = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            v1.to_str().expect("utf-8 workflow path"),
        ],
    );
    assert_eq!(
        first_step.get("effects_created").and_then(Value::as_u64),
        Some(1)
    );

    let revision = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--cancel",
            "keep",
            "--json",
        ],
    );
    let removed_agents = revision
        .pointer("/agent_impact/removed_agents_affecting_effects")
        .and_then(Value::as_array)
        .expect("removed agent impact list");
    assert_eq!(removed_agents.len(), 1, "{revision}");
    assert_eq!(
        removed_agents[0].get("agent").and_then(Value::as_str),
        Some("worker")
    );
    assert_eq!(
        removed_agents[0].get("status").and_then(Value::as_str),
        Some("queued")
    );
    assert_eq!(
        removed_agents[0]
            .get("program_version_id")
            .and_then(Value::as_str),
        Some(original_version)
    );

    let worker = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            instance_id,
            "--provider",
            "fixture",
        ],
    );
    assert_eq!(worker.get("ran_effects").and_then(Value::as_u64), Some(1));

    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    let old_effect = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("agent.tell"))
        .expect("old agent effect");
    assert_eq!(
        old_effect.get("status").and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        old_effect.get("program_version_id").and_then(Value::as_str),
        Some(original_version)
    );
    assert_eq!(
        old_effect.get("revision_epoch").and_then(Value::as_i64),
        Some(0)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn running_cancel_revision_requests_without_terminal_cancellation() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("running-cancel-v1");
    let v2 = temp_workflow_path("running-cancel-v2");
    fs::write(
        &v1,
        r#"
workflow RunningCancelRevision

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule start_work
  when started
  when worker is available
=> {
  tell worker "running work"
}
"#,
    )
    .expect("write v1 workflow");
    fs::write(
        &v2,
        r#"
workflow RunningCancelRevision

rule noop_v2
  when started
=> {
}
"#,
    )
    .expect("write v2 workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            v1.to_str().expect("utf-8 workflow path"),
        ],
    );
    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    let effect_id = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("agent.tell"))
        .and_then(|effect| effect.get("effect_id"))
        .and_then(Value::as_str)
        .expect("agent effect id")
        .to_owned();

    let mut store = SqliteStore::open(&store_path).expect("open store");
    store
        .start_run(RunStart {
            instance_id,
            effect_id: &effect_id,
            run_id: "run-running-cancel",
            provider: "fixture",
            worker_id: "worker-1",
            lease_id: "lease-running-cancel",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("run starts");

    let activation = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--cancel",
            "running",
            "--json",
        ],
    );
    let requested = activation
        .get("cancellation")
        .and_then(|cancellation| cancellation.get("request_cancel_effects"))
        .and_then(Value::as_array)
        .expect("request cancel effects");
    assert!(requested
        .iter()
        .any(|effect| effect.as_str() == Some(effect_id.as_str())));

    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    let running_effect = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("effect_id").and_then(Value::as_str) == Some(effect_id.as_str()))
        .expect("running effect");
    assert_eq!(
        running_effect.get("status").and_then(Value::as_str),
        Some("running")
    );
    assert_eq!(
        running_effect
            .get("cancel_requested")
            .and_then(Value::as_bool),
        Some(true)
    );

    let runs = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "runs",
            instance_id,
        ],
    );
    let run = runs
        .as_array()
        .expect("runs array")
        .iter()
        .find(|run| run.get("effect_id").and_then(Value::as_str) == Some(effect_id.as_str()))
        .expect("running run");
    assert_eq!(run.get("status").and_then(Value::as_str), Some("running"));
    assert_eq!(
        run.get("cancel_requested").and_then(Value::as_bool),
        Some(true)
    );

    let trace = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "trace",
            instance_id,
            "--check",
        ],
    );
    assert_eq!(
        trace
            .get("conformance")
            .and_then(|conformance| conformance.get("ok"))
            .and_then(Value::as_bool),
        Some(true)
    );
    let abstract_trace = trace
        .get("abstract_trace")
        .and_then(Value::as_array)
        .expect("abstract trace");
    assert!(abstract_trace.iter().any(|record| {
        record
            .get("event")
            .and_then(|event| event.get("type"))
            .and_then(Value::as_str)
            == Some("effect_cancellation_requested")
    }));
    assert!(!abstract_trace.iter().any(|record| {
        record
            .get("event")
            .and_then(|event| event.get("type"))
            .and_then(Value::as_str)
            == Some("effect_cancelled")
    }));

    let worker = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            instance_id,
        ],
    );
    assert_eq!(worker.get("ran_effects").and_then(Value::as_u64), Some(0));
    assert_eq!(
        worker
            .get("cancellation_diagnostics")
            .and_then(Value::as_u64),
        Some(1)
    );
    let diagnostics = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "diagnostics",
            instance_id,
        ],
    );
    let diagnostics = diagnostics.as_array().expect("diagnostics array");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.get("code").and_then(Value::as_str) == Some("provider.cancellation.unsupported")
            && diagnostic.get("effect_id").and_then(Value::as_str) == Some(effect_id.as_str())
            && diagnostic.get("run_id").and_then(Value::as_str) == Some("run-running-cancel")
    }));

    let mut store = SqliteStore::open(&store_path).expect("open store");
    store
        .complete_effect(EffectCompletion {
            instance_id,
            effect_id: &effect_id,
            run_id: "run-running-cancel",
            provider: "fixture",
            worker_id: "worker-1",
            status: "completed",
            exit_code: Some(0),
            summary: Some("late provider completion"),
            metadata_json: "{}",
            idempotency_key: Some("late-provider-completion-after-cancel-request"),
        })
        .expect("late completion succeeds");
    let requests = store
        .list_effect_cancellation_requests(instance_id)
        .expect("requests list");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].status, "terminal");
    drop(store);

    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    let completed_effect = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("effect_id").and_then(Value::as_str) == Some(effect_id.as_str()))
        .expect("completed effect");
    assert_eq!(
        completed_effect.get("status").and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        completed_effect
            .get("cancel_requested")
            .and_then(Value::as_bool),
        Some(false)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn operator_incident_bundle_has_stable_status_trace_and_diagnostics_shape() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("operator-incident-v1");
    let v2 = temp_workflow_path("operator-incident-v2");
    fs::write(
        &v1,
        r#"
workflow OperatorIncident

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule start_work
  when started
  when worker is available
=> {
  tell worker "running work"
}
"#,
    )
    .expect("write v1 workflow");
    fs::write(
        &v2,
        r#"
workflow OperatorIncident

rule noop_v2
  when started
=> {
}
"#,
    )
    .expect("write v2 workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            v1.to_str().expect("utf-8 workflow path"),
        ],
    );
    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    let effect_id = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("agent.tell"))
        .and_then(|effect| effect.get("effect_id"))
        .and_then(Value::as_str)
        .expect("agent effect id")
        .to_owned();

    let mut store = SqliteStore::open(&store_path).expect("open store");
    store
        .start_run(RunStart {
            instance_id,
            effect_id: &effect_id,
            run_id: "run-operator-incident",
            provider: "fixture",
            worker_id: "worker-1",
            lease_id: "lease-operator-incident",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("run starts");
    store
        .record_artifact(ArtifactRecord {
            run_id: "run-operator-incident",
            kind: "transcript",
            path: "provider://fixture/runs/run-operator-incident/transcript",
            content_hash: Some("sha256:operatorincident0000000000000000000000000000000000000000"),
            mime_type: Some("text/plain"),
        })
        .expect("artifact records");
    drop(store);

    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--cancel",
            "running",
            "--json",
        ],
    );
    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            instance_id,
        ],
    );

    let status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "status",
            instance_id,
        ],
    );
    let runs = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "runs",
            instance_id,
        ],
    );
    let diagnostics = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "diagnostics",
            instance_id,
        ],
    );
    let trace = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "trace",
            instance_id,
            "--check",
        ],
    );
    let artifacts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "artifacts",
            "run-operator-incident",
        ],
    );

    let run = runs
        .as_array()
        .expect("runs array")
        .iter()
        .find(|run| run.get("run_id").and_then(Value::as_str) == Some("run-operator-incident"))
        .expect("incident run");
    let diagnostic = diagnostics
        .as_array()
        .expect("diagnostics array")
        .iter()
        .find(|diagnostic| {
            diagnostic.get("code").and_then(Value::as_str)
                == Some("provider.cancellation.unsupported")
        })
        .expect("provider cancellation diagnostic");
    let artifact = artifacts
        .get("artifacts")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .expect("artifact metadata");

    let incident_bundle = json!({
        "status": {
            "instance_status": status.pointer("/instance/status").and_then(Value::as_str),
            "active_runs": status.get("active_run_count").and_then(Value::as_u64),
            "cancel_requests": status.get("cancellation_request_count").and_then(Value::as_u64),
            "recent_event_types": status
                .get("recent_events")
                .and_then(Value::as_array)
                .expect("recent events")
                .iter()
                .map(|event| event.get("event_type").and_then(Value::as_str).unwrap_or(""))
                .collect::<Vec<_>>(),
        },
        "run": {
            "status": run.get("status").and_then(Value::as_str),
            "provider": run.get("provider").and_then(Value::as_str),
            "cancel_requested": run.get("cancel_requested").and_then(Value::as_bool),
            "artifact_count": run.get("artifact_count").and_then(Value::as_u64),
        },
        "diagnostic": {
            "severity": diagnostic.get("severity").and_then(Value::as_str),
            "code": diagnostic.get("code").and_then(Value::as_str),
            "run_id": diagnostic.get("run_id").and_then(Value::as_str),
        },
        "trace": {
            "schema": trace.get("schema").and_then(Value::as_str),
            "conformance_ok": trace.pointer("/conformance/ok").and_then(Value::as_bool),
            "abstract_event_types": trace
                .get("abstract_trace")
                .and_then(Value::as_array)
                .expect("abstract trace")
                .iter()
                .map(|record| {
                    record
                        .get("event")
                        .and_then(|event| event.get("type"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                })
                .collect::<Vec<_>>(),
            "trace_run_artifact_count": trace
                .get("runs")
                .and_then(Value::as_array)
                .and_then(|runs| {
                    runs.iter().find(|run| {
                        run.get("run_id").and_then(Value::as_str)
                            == Some("run-operator-incident")
                    })
                })
                .and_then(|run| run.get("artifact_count"))
                .and_then(Value::as_u64),
        },
        "artifact": {
            "kind": artifact.get("kind").and_then(Value::as_str),
            "mime_type": artifact.get("mime_type").and_then(Value::as_str),
            "path": artifact.get("path").and_then(Value::as_str),
            "content_hash": artifact.get("content_hash").and_then(Value::as_str),
        },
    });
    assert_eq!(
        incident_bundle,
        json!({
            "status": {
                "instance_status": "running",
                "active_runs": 1,
                "cancel_requests": 1,
                "recent_event_types": [
                    "external.started",
                    "rule.committed",
                    "effect.run_started",
                    "workflow.revision_activated",
                    "effect.cancellation_requested"
                ],
            },
            "run": {
                "status": "running",
                "provider": "fixture",
                "cancel_requested": true,
                "artifact_count": 1,
            },
            "diagnostic": {
                "severity": "warning",
                "code": "provider.cancellation.unsupported",
                "run_id": "run-operator-incident",
            },
            "trace": {
                "schema": "whipplescript.local_trace.v0",
                "conformance_ok": true,
                "abstract_event_types": [
                    "effect_created",
                    "effect_claimed",
                    "run_started",
                    "revision_activated",
                    "effect_cancellation_requested"
                ],
                "trace_run_artifact_count": 1,
            },
            "artifact": {
                "kind": "transcript",
                "mime_type": "text/plain",
                "path": "provider://fixture/runs/run-operator-incident/transcript",
                "content_hash": "sha256:operatorincident0000000000000000000000000000000000000000",
            },
        })
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn running_cancel_supported_provider_acknowledges_cancellation() {
    running_cancel_supported_provider_acknowledges_cancellation_case(
        "fixture-cancellable",
        "before_terminal",
    );
    running_cancel_supported_provider_acknowledges_cancellation_case(
        "pi-main",
        "after_terminal_allowed",
    );
}

fn running_cancel_supported_provider_acknowledges_cancellation_case(
    provider: &str,
    expected_acknowledgement_order: &str,
) {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("running-cancel-ack-v1");
    let v2 = temp_workflow_path("running-cancel-ack-v2");
    fs::write(
        &v1,
        r#"
workflow RunningCancelAck

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule start_work
  when started
  when worker is available
=> {
  tell worker "running work"
}
"#,
    )
    .expect("write v1 workflow");
    fs::write(
        &v2,
        r#"
workflow RunningCancelAck

rule noop_v2
  when started
=> {
}
"#,
    )
    .expect("write v2 workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            v1.to_str().expect("utf-8 workflow path"),
        ],
    );
    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    let effect_id = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("agent.tell"))
        .and_then(|effect| effect.get("effect_id"))
        .and_then(Value::as_str)
        .expect("agent effect id")
        .to_owned();
    let mut store = SqliteStore::open(&store_path).expect("open store");
    store
        .start_run(RunStart {
            instance_id,
            effect_id: &effect_id,
            run_id: "run-cancellable-provider",
            provider,
            worker_id: "worker-1",
            lease_id: "lease-cancellable-provider",
            lease_expires_at: "2030-01-01T00:00:00Z",
            metadata_json: "{}",
        })
        .expect("run starts");
    drop(store);

    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--cancel",
            "running",
            "--json",
        ],
    );
    let worker = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            instance_id,
            "--provider",
            provider,
        ],
    );
    assert_eq!(worker.get("ran_effects").and_then(Value::as_u64), Some(0));
    assert_eq!(
        worker
            .get("cancellation_acknowledgements")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        worker
            .get("cancellation_diagnostics")
            .and_then(Value::as_u64),
        Some(0)
    );
    let duplicate_worker = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            instance_id,
            "--provider",
            provider,
        ],
    );
    assert_eq!(
        duplicate_worker
            .get("cancellation_acknowledgements")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        duplicate_worker
            .get("cancellation_diagnostics")
            .and_then(Value::as_u64),
        Some(0)
    );

    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    let cancelled = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("effect_id").and_then(Value::as_str) == Some(effect_id.as_str()))
        .expect("cancelled effect");
    assert_eq!(
        cancelled.get("status").and_then(Value::as_str),
        Some("cancelled")
    );
    assert_eq!(
        cancelled.get("cancel_requested").and_then(Value::as_bool),
        Some(false)
    );
    let store = SqliteStore::open(&store_path).expect("open store");
    let requests = store
        .list_effect_cancellation_requests(instance_id)
        .expect("requests list");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].status, "terminal");
    drop(store);

    let events = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            instance_id,
        ],
    );
    let terminal = events
        .as_array()
        .expect("events array")
        .iter()
        .find(|event| {
            event.get("event_type").and_then(Value::as_str) == Some("effect.terminal")
                && event.pointer("/payload/run_id").and_then(Value::as_str)
                    == Some("run-cancellable-provider")
        })
        .expect("terminal event exists");
    assert_eq!(
        events
            .as_array()
            .expect("events array")
            .iter()
            .filter(|event| {
                event.get("event_type").and_then(Value::as_str) == Some("effect.terminal")
                    && event.pointer("/payload/run_id").and_then(Value::as_str)
                        == Some("run-cancellable-provider")
            })
            .count(),
        1
    );
    assert_eq!(
        terminal
            .pointer("/payload/metadata/acknowledgement_order")
            .and_then(Value::as_str),
        Some(expected_acknowledgement_order)
    );
    assert_eq!(
        terminal
            .pointer("/payload/metadata/cancellation_depth")
            .and_then(Value::as_str),
        Some("native_stop")
    );

    let trace = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "trace",
            instance_id,
            "--check",
        ],
    );
    assert_eq!(
        trace
            .get("conformance")
            .and_then(|conformance| conformance.get("ok"))
            .and_then(Value::as_bool),
        Some(true)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn queued_cancel_revision_terminal_cancels_old_effect() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("queued-cancel-v1");
    let v2 = temp_workflow_path("queued-cancel-v2");
    fs::write(
        &v1,
        r#"
workflow QueuedCancelRevision

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule start_work
  when started
  when worker is available
=> {
  tell worker "queued work"
}
"#,
    )
    .expect("write v1 workflow");
    fs::write(
        &v2,
        r#"
workflow QueuedCancelRevision

rule noop_v2
  when started
=> {
}
"#,
    )
    .expect("write v2 workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            v1.to_str().expect("utf-8 workflow path"),
        ],
    );
    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    let effect_id = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("agent.tell"))
        .and_then(|effect| effect.get("effect_id"))
        .and_then(Value::as_str)
        .expect("agent effect id")
        .to_owned();

    let activation = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--cancel",
            "queued",
            "--json",
        ],
    );
    let terminal_cancelled = activation
        .get("cancellation")
        .and_then(|cancellation| cancellation.get("terminal_cancel_effects"))
        .and_then(Value::as_array)
        .expect("terminal cancel effects");
    assert!(terminal_cancelled
        .iter()
        .any(|effect| effect.as_str() == Some(effect_id.as_str())));

    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    let cancelled_effect = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("effect_id").and_then(Value::as_str) == Some(effect_id.as_str()))
        .expect("cancelled effect");
    assert_eq!(
        cancelled_effect.get("status").and_then(Value::as_str),
        Some("cancelled")
    );
    assert_eq!(
        cancelled_effect
            .get("cancel_requested")
            .and_then(Value::as_bool),
        Some(false)
    );
    let log = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            instance_id,
        ],
    );
    let events = log.as_array().expect("event log array");
    assert!(events.iter().any(|event| {
        event.get("event_type").and_then(Value::as_str) == Some("effect.terminal")
            && event
                .get("payload")
                .and_then(|payload| payload.get("effect_id"))
                .and_then(Value::as_str)
                == Some(effect_id.as_str())
            && event
                .get("payload")
                .and_then(|payload| payload.get("status"))
                .and_then(Value::as_str)
                == Some("cancelled")
    }));
    assert!(!events.iter().any(|event| {
        event.get("event_type").and_then(Value::as_str) == Some("effect.cancelled")
    }));

    let trace = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "trace",
            instance_id,
            "--check",
        ],
    );
    assert_eq!(
        trace
            .get("conformance")
            .and_then(|conformance| conformance.get("ok"))
            .and_then(Value::as_bool),
        Some(true)
    );
    let abstract_trace = trace
        .get("abstract_trace")
        .and_then(Value::as_array)
        .expect("abstract trace");
    assert!(abstract_trace.iter().any(|record| {
        record
            .get("event")
            .and_then(|event| event.get("type"))
            .and_then(Value::as_str)
            == Some("effect_cancelled")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn revise_dry_run_reports_incompatible_root_without_mutating() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("revise-incompatible-v1");
    let v2 = temp_workflow_path("revise-incompatible-v2");
    fs::write(
        &v1,
        r#"
workflow OriginalRoot

rule noop
  when started
=> {
}
"#,
    )
    .expect("write v1 workflow");
    fs::write(
        &v2,
        r#"
workflow OtherRoot

rule noop
  when started
=> {
}
"#,
    )
    .expect("write v2 workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    let report = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(
        report
            .get("compatibility")
            .and_then(|value| value.get("compatible"))
            .and_then(Value::as_bool),
        Some(false)
    );
    let diagnostics = report
        .get("compatibility")
        .and_then(|value| value.get("diagnostics"))
        .and_then(Value::as_array)
        .expect("diagnostics");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.get("code").and_then(Value::as_str) == Some("revision.root_workflow_changed")
    }));

    let blocked = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--json",
        ])
        .output()
        .expect("command runs");
    assert!(!blocked.status.success());
    let blocked_report: Value =
        serde_json::from_slice(&blocked.stdout).expect("blocked report is json");
    assert_eq!(
        blocked_report
            .get("compatibility")
            .and_then(|value| value.get("compatible"))
            .and_then(Value::as_bool),
        Some(false)
    );
    let persisted = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "diagnostics",
            instance_id,
        ],
    );
    let persisted = persisted.as_array().expect("diagnostics array");
    let root_diagnostic = persisted
        .iter()
        .find(|diagnostic| {
            diagnostic.get("code").and_then(Value::as_str) == Some("revision.root_workflow_changed")
                && diagnostic.get("subject_type").and_then(Value::as_str)
                    == Some("revision_compatibility")
        })
        .expect("persisted root compatibility diagnostic");
    let root_diagnostic_id = root_diagnostic
        .get("diagnostic_id")
        .and_then(Value::as_str)
        .expect("diagnostic id")
        .to_owned();

    let log = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            instance_id,
        ],
    );
    let rejected_event = log
        .as_array()
        .expect("log array")
        .iter()
        .find(|event| {
            event.get("event_type").and_then(Value::as_str) == Some("workflow.revision_rejected")
        })
        .expect("revision rejected event");
    let rejected_event_id = rejected_event
        .get("event_id")
        .and_then(Value::as_str)
        .expect("event id")
        .to_owned();
    assert_eq!(
        root_diagnostic.get("event_id").and_then(Value::as_str),
        Some(rejected_event_id.as_str())
    );

    let evidence = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "evidence",
            instance_id,
        ],
    );
    let evidence_items = evidence
        .get("evidence")
        .and_then(Value::as_array)
        .expect("evidence array");
    assert!(evidence_items.iter().any(|item| {
        item.get("kind").and_then(Value::as_str) == Some("workflow.revision.compatibility_rejected")
            && item.get("causation_id").and_then(Value::as_str) == Some(rejected_event_id.as_str())
    }));
    let evidence_links = evidence
        .get("links")
        .and_then(Value::as_array)
        .expect("evidence links");
    assert!(evidence_links.iter().any(|link| {
        link.get("target_type").and_then(Value::as_str) == Some("event")
            && link.get("target_id").and_then(Value::as_str) == Some(rejected_event_id.as_str())
            && link.get("relation").and_then(Value::as_str) == Some("rejected")
    }));
    assert!(evidence_links.iter().any(|link| {
        link.get("target_type").and_then(Value::as_str) == Some("diagnostic")
            && link.get("target_id").and_then(Value::as_str) == Some(root_diagnostic_id.as_str())
            && link.get("relation").and_then(Value::as_str) == Some("compatibility_diagnostic")
    }));

    let status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "status",
            instance_id,
            "--json",
        ],
    );
    assert_eq!(
        status
            .get("revisions")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn revise_dry_run_reports_contract_and_schema_source_spans() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("revise-span-v1");
    let v2 = temp_workflow_path("revise-span-v2");
    fs::write(
        &v1,
        r#"
workflow RevisionSpan {
  output done Result

  class Result {
    title string
  }

  class WorkItem {
    title string
  }

  rule seed
    when started
  => {
    record WorkItem {
      title "task"
    }
  }
}
"#,
    )
    .expect("write v1 workflow");
    let v2_source = r#"
workflow RevisionSpan {
  output done ChangedResult

  class ChangedResult {
    title string
  }

  class WorkItem {
    title int
    status string
  }
}
"#;
    fs::write(&v2, v2_source).expect("write v2 workflow");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let stepped = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "step",
            instance_id,
            "--program",
            v1.to_str().expect("utf-8 workflow path"),
            "--json",
        ],
    );
    assert_eq!(
        stepped.get("facts_created").and_then(Value::as_u64),
        Some(1)
    );

    let report = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--dry-run",
            "--json",
        ],
    );
    let diagnostics = report
        .pointer("/compatibility/diagnostics")
        .and_then(Value::as_array)
        .expect("diagnostics");
    let contract = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.get("code").and_then(Value::as_str) == Some("revision.contract_changed")
        })
        .expect("contract diagnostic");
    assert_eq!(
        contract
            .pointer("/source_span/construct")
            .and_then(Value::as_str),
        Some("workflow_contract")
    );
    let schema = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.get("code").and_then(Value::as_str)
                == Some("revision.active_fact_incompatible")
                && diagnostic.get("subject").and_then(Value::as_str) == Some("WorkItem")
        })
        .expect("schema diagnostic");
    assert_eq!(
        schema
            .pointer("/source_span/construct")
            .and_then(Value::as_str),
        Some("class")
    );
    assert_eq!(
        schema.pointer("/source_span/start").and_then(Value::as_u64),
        Some(v2_source.find("class WorkItem").expect("class offset") as u64)
    );
    let planned_diagnostics = report
        .pointer("/would_create/diagnostics")
        .and_then(Value::as_array)
        .expect("planned diagnostics");
    assert!(planned_diagnostics.iter().any(|diagnostic| {
        diagnostic.get("code").and_then(Value::as_str) == Some("revision.contract_changed")
            && diagnostic
                .pointer("/source_span/construct")
                .and_then(Value::as_str)
                == Some("workflow_contract")
    }));
    assert_eq!(
        report
            .pointer("/would_create/evidence/0/metadata/compatible")
            .and_then(Value::as_bool),
        Some(false)
    );

    let blocked = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            instance_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--json",
        ])
        .output()
        .expect("command runs");
    assert!(!blocked.status.success());

    let persisted = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "diagnostics",
            instance_id,
        ],
    );
    let persisted = persisted.as_array().expect("diagnostics array");
    assert!(persisted.iter().any(|diagnostic| {
        diagnostic.get("code").and_then(Value::as_str) == Some("revision.contract_changed")
            && diagnostic.get("subject_type").and_then(Value::as_str)
                == Some("revision_compatibility")
            && diagnostic
                .pointer("/source_span/construct")
                .and_then(Value::as_str)
                == Some("workflow_contract")
    }));
    assert!(persisted.iter().any(|diagnostic| {
        diagnostic.get("code").and_then(Value::as_str) == Some("revision.active_fact_incompatible")
            && diagnostic.get("subject_type").and_then(Value::as_str)
                == Some("revision_compatibility")
            && diagnostic
                .pointer("/source_span/construct")
                .and_then(Value::as_str)
                == Some("class")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn step_materializes_minimal_noop_fact() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("minimal-noop.whip");
    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "run",
            example.to_str().expect("utf-8 example path"),
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    let step = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            example.to_str().expect("utf-8 example path"),
        ],
    );
    assert_eq!(step.get("committed_rules").and_then(Value::as_u64), Some(1));
    assert_eq!(step.get("facts_created").and_then(Value::as_u64), Some(1));

    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("StartupSeen")
            && fact
                .get("value")
                .and_then(|value| value.get("state"))
                .and_then(Value::as_str)
                == Some("observed")
    }));

    let _ = fs::remove_file(store_path);
}

#[test]
fn dev_openclaw_lite_observes_heartbeat_and_files_work() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("openclaw-lite.whip");
    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            example.to_str().expect("utf-8 example path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let first_step = dev
        .get("steps")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .expect("first step");
    assert_eq!(
        first_step.get("facts_created").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        first_step.get("effects_created").and_then(Value::as_u64),
        Some(1)
    );
    let first_worker = dev
        .get("workers")
        .and_then(Value::as_array)
        .and_then(|workers| workers.first())
        .expect("first worker");
    assert_eq!(
        first_worker.get("ran_effects").and_then(Value::as_u64),
        Some(1)
    );

    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("Heartbeat"))
            .count(),
        1
    );
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("Plan"))
            .count(),
        1
    );
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("agent.turn.completed"))
            .count(),
        1
    );

    let _ = fs::remove_file(store_path);
}

#[test]
fn dev_native_fixture_records_provider_lifecycle_and_artifacts_from_source_workflow() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("native-fixture-provider-e2e");
    fs::write(
        &source_path,
        r#"
workflow NativeFixtureProviderE2E

agent worker {
  provider native-fixture
  profile "repo-writer"
  capacity 1
}

rule start_native_work
  when started
  when worker is available
=> {
  tell worker "create native fixture evidence"
}
"#,
    )
    .expect("write native fixture workflow");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "native-fixture",
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let workers = dev
        .get("workers")
        .and_then(Value::as_array)
        .expect("workers");
    let report_diagnostics = dev
        .get("diagnostics")
        .and_then(Value::as_array)
        .expect("dev report diagnostics");
    assert_eq!(
        dev.get("schema").and_then(Value::as_str),
        Some("whipplescript.dev_report.v0")
    );
    assert_eq!(
        dev.pointer("/provider_runs/summary/total")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        dev.pointer("/provider_runs/summary/artifact_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        dev.pointer("/provider_runs/groups/0/provider")
            .and_then(Value::as_str),
        Some("native-fixture")
    );
    assert_eq!(
        dev.pointer("/provider_artifacts/summary/total")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        dev.pointer("/provider_artifacts/groups/0/kind")
            .and_then(Value::as_str),
        Some("transcript")
    );
    assert_eq!(
        dev.pointer("/provider_artifacts/groups/0/mime_type")
            .and_then(Value::as_str),
        Some("text/plain")
    );
    let provider_artifact_items = dev
        .pointer("/provider_artifacts/items")
        .and_then(Value::as_array)
        .expect("provider artifact items");
    let transcript_artifact = provider_artifact_items
        .iter()
        .find(|item| item.get("kind").and_then(Value::as_str) == Some("transcript"))
        .expect("transcript artifact item");
    assert!(transcript_artifact
        .get("artifact_id")
        .and_then(Value::as_str)
        .is_some_and(|artifact_id| !artifact_id.is_empty()));
    assert!(transcript_artifact
        .get("run_id")
        .and_then(Value::as_str)
        .is_some_and(|run_id| !run_id.is_empty()));
    assert_eq!(
        transcript_artifact.get("mime_type").and_then(Value::as_str),
        Some("text/plain")
    );
    assert_eq!(
        dev.pointer("/provider_evidence/summary/total")
            .and_then(Value::as_u64),
        Some(8)
    );
    assert_eq!(
        dev.pointer("/provider_evidence/groups/0/kind")
            .and_then(Value::as_str),
        Some("agent.turn.native_event")
    );
    let provider_evidence_items = dev
        .pointer("/provider_evidence/items")
        .and_then(Value::as_array)
        .expect("provider evidence items");
    let native_event_evidence = provider_evidence_items
        .iter()
        .find(|item| item.get("kind").and_then(Value::as_str) == Some("agent.turn.native_event"))
        .expect("native event evidence item");
    assert_eq!(
        native_event_evidence
            .get("subject_type")
            .and_then(Value::as_str),
        Some("run")
    );
    assert!(native_event_evidence
        .get("subject_id")
        .and_then(Value::as_str)
        .is_some_and(|subject_id| !subject_id.is_empty()));
    assert!(native_event_evidence
        .get("summary")
        .and_then(Value::as_str)
        .is_some_and(|summary| !summary.is_empty()));
    assert_eq!(
        workers
            .iter()
            .map(|worker| worker
                .get("ran_effects")
                .and_then(Value::as_u64)
                .unwrap_or(0))
            .collect::<Vec<_>>(),
        vec![1, 0]
    );
    assert!(report_diagnostics.is_empty());

    let log = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            instance_id,
        ],
    );
    let events = log.as_array().expect("event array");
    for event_type in ["agent.turn.started", "agent.turn.artifact_captured"] {
        assert!(
            events.iter().any(|event| {
                event.get("event_type").and_then(Value::as_str) == Some(event_type)
                    && event
                        .get("payload")
                        .and_then(|payload| payload.get("provider"))
                        .and_then(Value::as_str)
                        == Some("native-fixture")
            }),
            "missing native lifecycle event {event_type}: {events:#?}"
        );
    }

    let runs = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "runs",
            instance_id,
        ],
    );
    let run = runs
        .as_array()
        .expect("runs array")
        .iter()
        .find(|run| run.get("provider").and_then(Value::as_str) == Some("native-fixture"))
        .expect("native fixture run");
    assert_eq!(run.get("status").and_then(Value::as_str), Some("completed"));
    assert_eq!(run.get("artifact_count").and_then(Value::as_u64), Some(1));
    let run_id = run.get("run_id").and_then(Value::as_str).expect("run id");

    let artifacts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "artifacts",
            run_id,
        ],
    );
    assert_eq!(
        artifacts
            .get("artifacts")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    let recover = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "recover",
            instance_id,
        ],
    );
    assert_eq!(
        recover.get("recovered_count").and_then(Value::as_u64),
        Some(0)
    );
    let replayed_log = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            instance_id,
        ],
    );
    let replayed_events = replayed_log.as_array().expect("replayed event array");
    assert_eq!(
        replayed_events
            .iter()
            .filter(|event| {
                event.get("event_type").and_then(Value::as_str) == Some("agent.turn.completed")
                    && event
                        .get("payload")
                        .and_then(|payload| payload.get("provider"))
                        .and_then(Value::as_str)
                        == Some("native-fixture")
            })
            .count(),
        1
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_native_provider_launch_failure_records_durable_boundary_failure() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("native-provider-unavailable");
    fs::write(
        &source_path,
        r#"
workflow NativeProviderUnavailable

agent worker {
  provider pi
  profile "repo-reader"
  capacity 1
}

rule start_native_work
  when started
  when worker is available
=> {
  tell worker "read-only native provider launch failure"
}
"#,
    )
    .expect("write unavailable native provider workflow");

    let output = Command::new(bin)
        .env(
            "WHIPPLESCRIPT_PI_RPC_COMMAND",
            "__whipplescript_missing_pi_rpc_command__",
        )
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "pi",
            "--until",
            "idle",
        ])
        .output()
        .expect("dev command runs");
    assert!(
        output.status.success(),
        "dev failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let dev = serde_json::from_slice::<Value>(&output.stdout).expect("dev json");
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let workers = dev
        .get("workers")
        .and_then(Value::as_array)
        .expect("workers");
    let report_diagnostics = dev
        .get("diagnostics")
        .and_then(Value::as_array)
        .expect("dev report diagnostics");
    assert_eq!(
        dev.get("schema").and_then(Value::as_str),
        Some("whipplescript.dev_report.v0")
    );
    assert_eq!(
        workers
            .iter()
            .map(|worker| worker
                .get("ran_effects")
                .and_then(Value::as_u64)
                .unwrap_or(0))
            .collect::<Vec<_>>(),
        vec![1, 0]
    );
    assert!(report_diagnostics.iter().any(|diagnostic| {
        diagnostic.get("code").and_then(Value::as_str) == Some("native_provider_failed")
            && diagnostic.get("subject_type").and_then(Value::as_str) == Some("effect")
            && diagnostic.get("event_id").and_then(Value::as_str).is_some()
            && diagnostic.get("run_id").and_then(Value::as_str).is_some()
    }));

    let runs = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "runs",
            instance_id,
        ],
    );
    let run = runs
        .as_array()
        .expect("runs array")
        .iter()
        .find(|run| run.get("provider").and_then(Value::as_str) == Some("pi"))
        .expect("pi run");
    assert_eq!(run.get("status").and_then(Value::as_str), Some("failed"));

    let log = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            instance_id,
        ],
    );
    let events = log.as_array().expect("event array");
    assert!(events.iter().any(|event| {
        event.get("event_type").and_then(Value::as_str) == Some("agent.turn.failed")
            && event
                .get("payload")
                .and_then(|payload| payload.get("provider_event_type"))
                .and_then(Value::as_str)
                == Some("whip.native.boundary_error.provider_health_unavailable")
    }));

    let diagnostics = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "diagnostics",
            instance_id,
        ],
    );
    let diagnostics = diagnostics.as_array().expect("diagnostics array");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.get("code").and_then(Value::as_str) == Some("native_provider_failed")
            && diagnostic.get("subject_type").and_then(Value::as_str) == Some("effect")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_native_fixture_stress_records_one_terminal_per_effect() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("native-fixture-stress");
    fs::write(
        &source_path,
        r#"
workflow NativeFixtureStress

agent worker {
  provider native-fixture
  profile "repo-writer"
  capacity 2
}

rule start_native_work
  when started
  when worker is available
=> {
  tell worker "native fixture stress one"
  tell worker "native fixture stress two"
  tell worker "native fixture stress three"
  tell worker "native fixture stress four"
}
"#,
    )
    .expect("write native fixture stress workflow");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "native-fixture",
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let ran_effects = dev
        .get("workers")
        .and_then(Value::as_array)
        .expect("workers")
        .iter()
        .map(|worker| {
            worker
                .get("ran_effects")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
        .sum::<u64>();
    assert_eq!(ran_effects, 4);

    let runs = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "runs",
            instance_id,
        ],
    );
    let native_runs = runs
        .as_array()
        .expect("runs array")
        .iter()
        .filter(|run| run.get("provider").and_then(Value::as_str) == Some("native-fixture"))
        .collect::<Vec<_>>();
    assert_eq!(native_runs.len(), 4);
    for run in &native_runs {
        assert_eq!(run.get("status").and_then(Value::as_str), Some("completed"));
        assert_eq!(run.get("artifact_count").and_then(Value::as_u64), Some(1));
    }

    let log = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            instance_id,
        ],
    );
    let events = log.as_array().expect("event array");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.get("event_type").and_then(Value::as_str)
                == Some("agent.turn.completed")
                && event
                    .get("payload")
                    .and_then(|payload| payload.get("provider"))
                    .and_then(Value::as_str)
                    == Some("native-fixture"))
            .count(),
        4
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.get("event_type").and_then(Value::as_str)
                == Some("agent.turn.artifact_captured")
                && event
                    .get("payload")
                    .and_then(|payload| payload.get("provider"))
                    .and_then(Value::as_str)
                    == Some("native-fixture"))
            .count(),
        4
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.get("event_type").and_then(Value::as_str)
                == Some("effect.terminal")
                && event
                    .get("payload")
                    .and_then(|payload| payload.get("provider"))
                    .and_then(Value::as_str)
                    == Some("native-fixture"))
            .count(),
        4
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_fixture_failure_reaches_event_stream() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("ralph.whip");
    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            example.to_str().expect("utf-8 example path"),
            "--provider",
            "fixture",
            "--fail",
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    let log = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            instance_id,
        ],
    );
    let events = log.as_array().expect("event array");
    assert!(events.iter().any(|event| {
        event.get("event_type").and_then(Value::as_str) == Some("effect.terminal")
            && event
                .get("payload")
                .and_then(|payload| payload.get("status"))
                .and_then(Value::as_str)
                == Some("failed")
            && event
                .get("payload")
                .and_then(|payload| payload.get("metadata"))
                .and_then(|metadata| metadata.get("failure"))
                .and_then(|failure| failure.get("phase"))
                .and_then(Value::as_str)
                == Some("provider.exit.failed")
    }));
    assert!(events.iter().any(|event| {
        event.get("event_type").and_then(Value::as_str) == Some("agent.turn.failed")
            && event
                .get("payload")
                .and_then(|payload| payload.get("failure"))
                .and_then(|failure| failure.get("error_kind"))
                .and_then(Value::as_str)
                == Some("nonzero_exit")
    }));
    let diagnostics = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "diagnostics",
            instance_id,
        ],
    );
    let diagnostics = diagnostics.as_array().expect("diagnostics array");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.get("code").and_then(Value::as_str) == Some("nonzero_exit")
            && diagnostic.get("subject_type").and_then(Value::as_str) == Some("effect")
            && diagnostic
                .get("subject_id")
                .and_then(Value::as_str)
                .is_some()
            && diagnostic.get("event_id").and_then(Value::as_str).is_some()
            && diagnostic.get("run_id").and_then(Value::as_str).is_some()
            && diagnostic
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains("fixture failed"))
    }));

    let _ = fs::remove_file(store_path);
}

#[test]
fn dev_coerce_failure_releases_human_ask_dependency() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("coerce-failure");
    fs::write(
        &workflow_path,
        r#"
workflow CoerceFailure

class WorkItem {
  title string
  body string
}

class MessageClassification {
  summary string
  confidence float
}

coerce classifyMessage(title string, body string) -> MessageClassification {
  prompt "Classify"
}

rule seed
  when started
=> {
  record WorkItem {
    title "One"
    body "Two"
  }
}

rule classify_request
  when WorkItem as request
=> {
  coerce classifyMessage(request.title, request.body) as classification

  after classification fails {
    askHuman """
    Failed to classify {{ request.title }}
    """
  }
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--provider",
            "fixture",
            "--fail",
            "--until",
            "idle",
        ],
    );
    let workers = dev
        .get("workers")
        .and_then(Value::as_array)
        .expect("workers");
    assert_eq!(
        dev.get("schema").and_then(Value::as_str),
        Some("whipplescript.dev_report.v0")
    );
    assert_eq!(
        workers
            .iter()
            .map(|worker| worker
                .get("ran_effects")
                .and_then(Value::as_u64)
                .unwrap_or(0))
            .collect::<Vec<_>>(),
        vec![1, 1, 0]
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("baml.coerce.failed")));
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("human.ask.created")));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_reports_human_prompt_content_type_in_assertion_effect_matches() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("human-prompt-content-type");
    fs::write(
        &workflow_path,
        r#"
workflow HumanPromptContentType

@acceptance
assert count(effect kind human.ask where status == completed) == 1

rule start
  when started
=> {
  askHuman """application/json
  {
    "question": "Approve this release?"
  }
  """
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
            "--include-tag",
            "acceptance",
        ],
    );
    assert_eq!(
        dev.pointer("/executable_spec/status")
            .and_then(Value::as_str),
        Some("passed")
    );
    let assertions = dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions");
    assert_eq!(assertions.len(), 1);
    assert_eq!(
        assertions[0]
            .pointer("/reads/0/kind")
            .and_then(Value::as_str),
        Some("effect")
    );
    assert_eq!(
        assertions[0]
            .pointer("/reads/0/head")
            .and_then(Value::as_str),
        Some("kind human.ask")
    );
    assert_eq!(
        assertions[0]
            .pointer("/reads/0/matches/0/prompt_content_type")
            .and_then(Value::as_str),
        Some("application/json")
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_queue_claim_success_releases_agent_turn_dependency() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let items_path = temp_store_path();
    let workflow_path = temp_workflow_path("queue-claim");
    fs::write(
        &workflow_path,
        r#"
@service
workflow QueueClaim

queue backlog {
  tracker builtin
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule seed
  when started
=> {
  file item into backlog {
    title "Fix it"
    body "Please"
  }
}

rule start_item
  when backlog has ready item as item
  when worker is available
=> {
  claim item as lease

  after lease succeeds {
    tell worker as turn """
    Implement {{ item.title }}
    """
  }

  after turn succeeds as outcome {
    finish item {
      summary outcome.summary
    }
  }
}
"#,
    )
    .expect("workflow writes");

    let output = Command::new(bin)
        .env("WHIPPLESCRIPT_ITEMS_STORE", &items_path)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ])
        .output()
        .expect("command runs");
    assert!(
        output.status.success(),
        "dev failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let dev: Value =
        serde_json::from_str(&stdout[stdout.find('{').expect("json")..]).expect("dev json");
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("queue.claim.completed")));
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("agent.turn.completed")));
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("queue.finish.completed")));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(items_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_coerce_success_materializes_after_record() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("coerce-success");
    fs::write(
        &workflow_path,
        r#"
workflow CoerceSuccess

class WorkItem {
  title string
  body string
}

class MessageClassification {
  priority string
  summary string
  confidence float
}

class ClassifiedMessage {
  request WorkItem
  classification MessageClassification
}

coerce classifyMessage(title string, body string) -> MessageClassification {
  prompt "Classify"
}

rule seed
  when started
=> {
  record WorkItem {
    title "One"
    body "Two"
  }
}

rule classify_request
  when WorkItem as request
=> {
  coerce classifyMessage(request.title, request.body) as classification

  after classification succeeds {
    record ClassifiedMessage {
      request request
      classification classification
    }
  }
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let workers = dev
        .get("workers")
        .and_then(Value::as_array)
        .expect("workers");
    assert_eq!(
        workers
            .iter()
            .map(|worker| worker
                .get("ran_effects")
                .and_then(Value::as_u64)
                .unwrap_or(0))
            .collect::<Vec<_>>(),
        vec![1, 0, 0]
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("baml.coerce.succeeded")));
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("ClassifiedMessage")
            && fact
                .get("value")
                .and_then(|value| value.get("classification"))
                .and_then(|classification| classification.get("summary"))
                .and_then(Value::as_str)
                == Some("Fixture classification")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_capability_call_fixture_releases_agent_turn_dependency() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("capability-call");
    fs::write(
        &workflow_path,
        r#"
workflow CapabilityCall

use memory

class WorkItem {
  title string
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule seed
  when started
=> {
  record WorkItem {
    title "Remember this"
  }
}

rule recall_before_work
  when WorkItem as item
  when worker is available
=> {
  call memory.query for item as context

  after context succeeds {
    tell worker """
    Use the recalled context for {{ item.title }}.
    """
  }
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let workers = dev
        .get("workers")
        .and_then(Value::as_array)
        .expect("workers");
    assert_eq!(
        workers
            .iter()
            .map(|worker| worker
                .get("ran_effects")
                .and_then(Value::as_u64)
                .unwrap_or(0))
            .collect::<Vec<_>>(),
        vec![1, 1, 0]
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("capability.call.succeeded")));
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("agent.turn.completed")));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn check_rejects_removed_emit_statement() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let workflow_path = temp_workflow_path("event-emit-removed");
    fs::write(
        &workflow_path,
        r#"
workflow EventEmit

class Tick {
  status "ready"
}

rule emit_heartbeat
  when Tick as tick where tick.status == "ready"
=> {
  emit openclaw.heartbeat as heartbeat
}
"#,
    )
    .expect("workflow writes");

    let output = Command::new(bin)
        .args([
            "check",
            workflow_path.to_str().expect("utf-8 workflow path"),
        ])
        .output()
        .expect("command runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("`emit` was removed from the language"),
        "{stderr}"
    );

    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_complete_terminal_action_marks_instance_completed() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("workflow-complete");
    fs::write(
        &workflow_path,
        r#"
workflow WorkflowComplete {
  output result CompletionResult
  failure error CompletionFailure

  class CompletionResult {
    status "ok"
  }

  class CompletionFailure {
    reason string
  }

  rule complete_immediately
    when started
  => {
    complete result {
      status "ok"
    }
  }
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "status",
            instance_id,
        ],
    );
    assert_eq!(
        status
            .get("instance")
            .and_then(|instance| instance.get("status"))
            .and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        status
            .get("workflow_terminal")
            .and_then(|terminal| terminal.get("status"))
            .and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        status
            .get("workflow_terminal")
            .and_then(|terminal| terminal.get("name"))
            .and_then(Value::as_str),
        Some("result")
    );
    let events = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            instance_id,
        ],
    );
    assert!(
        events
            .as_array()
            .expect("events array")
            .iter()
            .any(|event| event.get("event_type").and_then(Value::as_str)
                == Some("workflow.completed"))
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_validates_and_seeds_declared_workflow_inputs() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("workflow-input");
    fs::write(
        &workflow_path,
        r#"
workflow WorkflowInput {
  input phase PhaseRequest

  class PhaseRequest {
    phaseId string
    title string
  }

  class PhaseAccepted {
    phaseId string
    title string
  }

  rule accept_input
    when PhaseRequest as phase
  => {
    record PhaseAccepted {
      phaseId phase.phaseId
      title phase.title
    }
  }
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--input",
            r#"{"phase":{"phaseId":"p1","title":"Review parser"}}"#,
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("PhaseRequest")
            && fact.get("key").and_then(Value::as_str) == Some("phase")
            && fact
                .get("value")
                .and_then(|value| value.get("title"))
                .and_then(Value::as_str)
                == Some("Review parser")
    }));
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("PhaseAccepted")
            && fact
                .get("value")
                .and_then(|value| value.get("phaseId"))
                .and_then(Value::as_str)
                == Some("p1")
    }));

    let invalid = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--input",
            r#"{"phase":{"phaseId":"p2"}}"#,
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--until",
            "idle",
        ])
        .output()
        .expect("command runs");
    assert!(!invalid.status.success());
    let stderr = String::from_utf8_lossy(&invalid.stderr);
    assert!(
        stderr.contains("invalid workflow input") && stderr.contains("phase.title is required"),
        "{stderr}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_runs_rule_generated_by_pattern_application() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("pattern-application");
    fs::write(
        &workflow_path,
        r#"
pattern RecordSeen<Input, Output> {
  rule dispatch
    when Input as item
  => {
    done item -> record Output {
      title item.title
    }
  }
}

workflow PatternApplication {
  input task Task

  class Task {
    title string
  }

  class TaskSeen {
    title string
  }

  apply RecordSeen<Task, TaskSeen> as taskSeen {
  }
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--input",
            r#"{"task":{"title":"Pattern smoke"}}"#,
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("TaskSeen")
            && fact
                .get("value")
                .and_then(|value| value.get("title"))
                .and_then(Value::as_str)
                == Some("Pattern smoke")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_creates_workflow_invoke_effect() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("workflow-invoke");
    fs::write(
        &workflow_path,
        r#"
workflow Parent {
  input task Task

  class Task {
    title string
  }

  class ParentDone {
    title string
  }

  rule dispatch
    when Task as task
  => {
    invoke Child { task { title task.title } } as child

    after child succeeds as result {
      record ParentDone {
        title result.title
      }
    }
  }
}

workflow Child {
  input task Task
  output result ChildResult

  class Task {
    title string
  }

  class ChildResult {
    title string
  }

  rule complete_child
    when Task as task
  => {
    complete result {
      title task.title
    }
  }
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--input",
            r#"{"task":{"title":"Invoke smoke"}}"#,
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    let effects = effects.as_array().expect("effects array");
    assert!(effects.iter().any(|effect| {
        effect.get("kind").and_then(Value::as_str) == Some("workflow.invoke")
            && effect.get("target").and_then(Value::as_str) == Some("Child")
            && effect
                .get("input")
                .and_then(|input| input.get("input"))
                .and_then(|input| input.get("task"))
                .and_then(|task| task.get("title"))
                .and_then(Value::as_str)
                == Some("Invoke smoke")
    }));
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("workflow.invoke.succeeded")
            && fact
                .get("value")
                .and_then(|value| value.get("value"))
                .and_then(|value| value.get("title"))
                .and_then(Value::as_str)
                == Some("Invoke smoke")
    }));
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("ParentDone")
            && fact
                .get("value")
                .and_then(|value| value.get("title"))
                .and_then(Value::as_str)
                == Some("Invoke smoke")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn worker_resumes_running_workflow_invocation() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("workflow-invoke-resume");
    fs::write(
        &workflow_path,
        r#"
workflow Parent {
  input task Task

  class Task {
    title string
  }

  class ParentDone {
    title string
  }

  rule dispatch
    when Task as task
  => {
    invoke Child { task { title task.title } } as child

    after child succeeds as result {
      record ParentDone {
        title result.title
      }
    }
  }
}

workflow Child {
  input task Task
  output result ChildResult

  class Task {
    title string
  }

  class ChildResult {
    title string
  }

  rule complete_child
    when Task as task
  => {
    complete result {
      title task.title
    }
  }
}
"#,
    )
    .expect("workflow writes");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--input",
            r#"{"task":{"title":"Resume smoke"}}"#,
            "--json",
            "run",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let first_step = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );
    assert_eq!(
        first_step.get("effects_created").and_then(Value::as_u64),
        Some(1)
    );

    let first_worker = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            instance_id,
            "--program",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
            "--max-child-iterations",
            "0",
        ],
    );
    assert_eq!(
        first_worker.get("ran_effects").and_then(Value::as_u64),
        Some(1)
    );
    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    assert!(effects.as_array().expect("effects").iter().any(|effect| {
        effect.get("kind").and_then(Value::as_str) == Some("workflow.invoke")
            && effect.get("status").and_then(Value::as_str) == Some("running")
    }));
    let instances = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "instances",
        ],
    );
    let instances = instances.as_array().expect("instances");
    assert_eq!(instances.len(), 2);
    let child_instance_id = instances
        .iter()
        .filter_map(|instance| instance.get("instance_id").and_then(Value::as_str))
        .find(|candidate| *candidate != instance_id)
        .expect("child instance id");
    let parent_status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "status",
            instance_id,
        ],
    );
    let child_status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "status",
            child_instance_id,
        ],
    );
    assert_eq!(
        parent_status
            .get("workflow_invocations")
            .and_then(|value| value.get("children"))
            .and_then(Value::as_array)
            .and_then(|children| children.first())
            .and_then(|child| child.get("child_instance_id"))
            .and_then(Value::as_str),
        Some(child_instance_id)
    );
    assert_eq!(
        child_status
            .get("workflow_invocations")
            .and_then(|value| value.get("parent"))
            .and_then(|parent| parent.get("parent_instance_id"))
            .and_then(Value::as_str),
        Some(instance_id)
    );

    let second_worker = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            instance_id,
            "--program",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );
    assert_eq!(
        second_worker.get("ran_effects").and_then(Value::as_u64),
        Some(1)
    );
    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    assert!(effects.as_array().expect("effects").iter().any(|effect| {
        effect.get("kind").and_then(Value::as_str) == Some("workflow.invoke")
            && effect.get("status").and_then(Value::as_str) == Some("completed")
    }));

    let second_step = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );
    assert_eq!(
        second_step.get("facts_created").and_then(Value::as_u64),
        Some(1)
    );
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("workflow.invoke.succeeded")
            && fact
                .get("value")
                .and_then(|value| value.get("value"))
                .and_then(|value| value.get("title"))
                .and_then(Value::as_str)
                == Some("Resume smoke")
    }));
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("ParentDone")
            && fact
                .get("value")
                .and_then(|value| value.get("title"))
                .and_then(Value::as_str)
                == Some("Resume smoke")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn worker_preserves_child_invocation_links_after_parent_revision() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("workflow-invoke-parent-revision-v1");
    let v2 = temp_workflow_path("workflow-invoke-parent-revision-v2");
    fs::write(
        &v1,
        r#"
workflow Parent {
  input task Task

  class Task {
    title string
  }

  class ParentDone {
    title string
  }

  rule dispatch
    when Task as task
  => {
    invoke Child { task { title task.title } } as child

    after child succeeds as result {
      record ParentDone {
        title result.title
      }
    }
  }
}

workflow Child {
  input task Task
  output result ChildResult

  class Task {
    title string
  }

  class ChildResult {
    title string
  }

  rule complete_child
    when Task as task
  => {
    complete result {
      title task.title
    }
  }
}
"#,
    )
    .expect("v1 workflow writes");
    fs::write(
        &v2,
        r#"
workflow Parent {
  input task Task

  class Task {
    title string
  }

  class ParentDone {
    title string
  }

  class RevisionMarker {
    version string
  }

  rule dispatch
    when Task as task
  => {
    invoke Child { task { title task.title } } as child

    after child succeeds as result {
      record ParentDone {
        title result.title
      }
    }
  }
}

workflow Child {
  input task Task
  output result ChildResult

  class Task {
    title string
  }

  class ChildResult {
    title string
  }

  rule complete_child
    when Task as task
  => {
    complete result {
      title task.title
    }
  }
}
"#,
    )
    .expect("v2 workflow writes");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--input",
            r#"{"task":{"title":"Revision link"}}"#,
            "--json",
            "run",
            v1.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );
    let parent_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("parent instance id");
    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            parent_id,
            "--program",
            v1.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );
    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            parent_id,
            "--program",
            v1.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
            "--max-child-iterations",
            "0",
        ],
    );
    let instances = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "instances",
        ],
    );
    let child_id = instances
        .as_array()
        .expect("instances")
        .iter()
        .filter_map(|instance| instance.get("instance_id").and_then(Value::as_str))
        .find(|candidate| *candidate != parent_id)
        .expect("child instance id")
        .to_owned();

    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "revise",
            parent_id,
            v2.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
            "--json",
        ],
    );

    let parent_status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "status",
            parent_id,
        ],
    );
    let invocation = parent_status
        .pointer("/workflow_invocations/children/0")
        .expect("parent invocation link");
    assert_eq!(
        invocation.get("child_instance_id").and_then(Value::as_str),
        Some(child_id.as_str())
    );
    assert_eq!(
        invocation
            .get("parent_revision_epoch")
            .and_then(Value::as_i64),
        Some(0)
    );
    assert_eq!(
        invocation
            .get("parent_active_revision_epoch")
            .and_then(Value::as_i64),
        Some(1)
    );
    assert_eq!(
        invocation
            .get("child_revision_epoch")
            .and_then(Value::as_i64),
        Some(0)
    );
    assert_eq!(
        invocation
            .get("child_active_revision_epoch")
            .and_then(Value::as_i64),
        Some(0)
    );
    assert_eq!(
        invocation.get("status").and_then(Value::as_str),
        Some("running")
    );

    let worker = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            parent_id,
            "--program",
            v1.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );
    assert_eq!(worker.get("ran_effects").and_then(Value::as_u64), Some(1));
    let repeat_worker = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            parent_id,
            "--program",
            v1.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );
    assert_eq!(
        repeat_worker.get("ran_effects").and_then(Value::as_u64),
        Some(0)
    );

    let parent_status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "status",
            parent_id,
        ],
    );
    assert_eq!(
        parent_status
            .pointer("/workflow_invocations/children/0/status")
            .and_then(Value::as_str),
        Some("completed")
    );
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            parent_id,
        ],
    );
    let success_count = facts
        .as_array()
        .expect("facts")
        .iter()
        .filter(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("workflow.invoke.succeeded")
        })
        .count();
    assert_eq!(success_count, 1);

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn worker_projects_cancelled_child_workflow_invocation() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("workflow-invoke-cancel");
    fs::write(
        &workflow_path,
        r#"
workflow Parent {
  input task Task

  class Task {
    title string
  }

  class ParentCancelled {
    reason string
  }

  rule dispatch
    when Task as task
  => {
    invoke Child { task { title task.title } } as child

    after child completes {
      case child {
        Completed result => {
          record ParentCancelled {
            reason result.title
          }
        }
        Failed failure => {
          record ParentCancelled {
            reason failure.reason
          }
        }
        TimedOut timeout => {
          record ParentCancelled {
            reason timeout.summary
          }
        }
        Cancelled cancel => {
          record ParentCancelled {
            reason cancel.summary
          }
        }
      }
    }
  }
}

workflow Child {
  input task Task
  output result ChildResult

  class Task {
    title string
  }

  class ChildResult {
    title string
  }

  class MissingFact {
    title string
  }

  rule wait_forever
    when MissingFact as missing
  => {
    complete result {
      title missing.title
    }
  }
}
"#,
    )
    .expect("workflow writes");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--input",
            r#"{"task":{"title":"Cancel smoke"}}"#,
            "--json",
            "run",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );
    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            instance_id,
            "--program",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
            "--max-child-iterations",
            "0",
        ],
    );
    let instances = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "instances",
        ],
    );
    let child_id = instances
        .as_array()
        .expect("instances")
        .iter()
        .filter_map(|instance| instance.get("instance_id").and_then(Value::as_str))
        .find(|candidate| *candidate != instance_id)
        .expect("child instance id");
    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "cancel",
            child_id,
        ],
    );
    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "worker",
            instance_id,
            "--program",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );

    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    assert!(effects.as_array().expect("effects").iter().any(|effect| {
        effect.get("kind").and_then(Value::as_str) == Some("workflow.invoke")
            && effect.get("status").and_then(Value::as_str) == Some("cancelled")
    }));
    run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            instance_id,
            "--program",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
        ],
    );
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("workflow.invoke.cancelled")
            && fact
                .get("value")
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str)
                == Some("cancelled")
    }));
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("ParentCancelled")
            && fact
                .get("value")
                .and_then(|value| value.get("reason"))
                .and_then(Value::as_str)
                == Some("child workflow cancelled")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_projects_failed_child_workflow_invocation() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("workflow-invoke-fail");
    fs::write(
        &workflow_path,
        r#"
workflow Parent {
  input task Task

  class Task {
    title string
  }

  class ParentBlocked {
    reason string
  }

  rule dispatch
    when Task as task
  => {
    invoke Child { task { title task.title } } as child

    after child fails as failure {
      record ParentBlocked {
        reason failure.reason
      }
    }
  }
}

workflow Child {
  input task Task
  failure error ChildFailure

  class Task {
    title string
  }

  class ChildFailure {
    reason string
  }

  rule fail_child
    when Task as task
  => {
    fail error {
      reason task.title
    }
  }
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--input",
            r#"{"task":{"title":"Needs revision"}}"#,
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("workflow.invoke.failed")
            && fact
                .get("value")
                .and_then(|value| value.get("value"))
                .and_then(|value| value.get("reason"))
                .and_then(Value::as_str)
                == Some("Needs revision")
    }));
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("ParentBlocked")
            && fact
                .get("value")
                .and_then(|value| value.get("reason"))
                .and_then(Value::as_str)
                == Some("Needs revision")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_projects_timed_out_child_workflow_invocation() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("workflow-invoke-timeout");
    fs::write(
        &workflow_path,
        r#"
workflow Parent {
  input task Task

  class Task {
    title string
  }

  class ParentBlocked {
    reason string
  }

  rule dispatch
    when Task as task
  => {
    invoke Child { task { title task.title } } as child

    after child fails as failure {
      record ParentBlocked {
        reason failure.reason
      }
    }
  }
}

workflow Child {
  input task Task
  output result ChildResult

  class Task {
    title string
  }

  class ChildResult {
    title string
  }

  class MissingFact {
    title string
  }

  rule never_ready
    when MissingFact as missing
  => {
    complete result {
      title missing.title
    }
  }
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--input",
            r#"{"task":{"title":"Eventually timeout"}}"#,
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let effects = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    assert!(effects.as_array().expect("effects").iter().any(|effect| {
        effect.get("kind").and_then(Value::as_str) == Some("workflow.invoke")
            && effect.get("status").and_then(Value::as_str) == Some("timed_out")
    }));
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("workflow.invoke.timed_out")
            && fact
                .get("value")
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str)
                == Some("timed_out")
    }));
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("ParentBlocked")
            && fact
                .get("value")
                .and_then(|value| value.get("reason"))
                .and_then(Value::as_str)
                .is_some_and(|reason| reason.contains("did not reach terminal state"))
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_fail_terminal_action_marks_instance_failed() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("workflow-fail");
    fs::write(
        &workflow_path,
        r#"
workflow WorkflowFail {
  output result CompletionResult
  failure error CompletionFailure

  class CompletionResult {
    status "ok"
  }

  class CompletionFailure {
    reason string
  }

  rule fail_immediately
    when started
  => {
    fail error {
      reason "blocked"
    }
  }
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "status",
            instance_id,
        ],
    );
    assert_eq!(
        status
            .get("instance")
            .and_then(|instance| instance.get("status"))
            .and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        status
            .get("workflow_terminal")
            .and_then(|terminal| terminal.get("status"))
            .and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(status.get("failure_count").and_then(Value::as_i64), Some(0));
    let events = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            instance_id,
        ],
    );
    assert!(events
        .as_array()
        .expect("events array")
        .iter()
        .any(|event| event.get("event_type").and_then(Value::as_str) == Some("workflow.failed")));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_provider_language_rehydrates_after_bound_baml_arguments() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("provider-language-e2e.whip");
    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            example.to_str().expect("utf-8 example path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let workers = dev
        .get("workers")
        .and_then(Value::as_array)
        .expect("workers");
    assert_eq!(
        workers
            .iter()
            .map(|worker| worker
                .get("ran_effects")
                .and_then(Value::as_u64)
                .unwrap_or(0))
            .collect::<Vec<_>>(),
        vec![6, 6, 0, 0]
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let evidence = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "evidence",
            instance_id,
        ],
    );
    let evidence_items = evidence
        .get("evidence")
        .and_then(Value::as_array)
        .expect("evidence array");
    let baml = evidence_items
        .iter()
        .find(|item| item.get("kind").and_then(Value::as_str) == Some("baml.coerce.provider"))
        .expect("baml provider evidence");
    let arguments = baml
        .get("metadata")
        .and_then(|metadata| metadata.get("arguments"))
        .expect("baml arguments");
    assert_eq!(
        arguments.get("redacted").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        arguments.pointer("/shape/type").and_then(Value::as_str),
        Some("object")
    );
    let arguments_json = arguments.to_string();
    assert!(!arguments_json.contains("target/dogfood/language/codex-french.txt"));
    assert!(!arguments_json.contains("fixture completed"));

    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("LanguageE2EResult")
            && fact
                .get("value")
                .and_then(|value| value.get("review"))
                .and_then(|review| review.get("isTargetLanguage"))
                .and_then(Value::as_bool)
                == Some(true)
    }));

    let _ = fs::remove_file(store_path);
}

#[test]
fn dev_provider_language_e2e_runs_agent_table_and_baml_reviews() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("provider-language-e2e.whip");
    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            example.to_str().expect("utf-8 example path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let workers = dev
        .get("workers")
        .and_then(Value::as_array)
        .expect("workers");
    assert_eq!(
        dev.get("schema").and_then(Value::as_str),
        Some("whipplescript.dev_report.v0")
    );
    assert_eq!(
        workers
            .iter()
            .map(|worker| worker
                .get("ran_effects")
                .and_then(Value::as_u64)
                .unwrap_or(0))
            .collect::<Vec<_>>(),
        vec![6, 6, 0, 0]
    );
    let assertions = dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions");
    assert_eq!(assertions.len(), 6);
    assert!(assertions
        .iter()
        .all(|assertion| assertion.get("passed").and_then(Value::as_bool) == Some(true)));
    assert!(assertions.iter().all(|assertion| assertion
        .get("tags")
        .and_then(Value::as_array)
        .is_some_and(|tags| tags.iter().any(|tag| tag.as_str() == Some("acceptance")))));
    assert!(assertions.iter().all(|assertion| assertion
        .get("target_id")
        .and_then(Value::as_str)
        .is_some_and(|target_id| !target_id.is_empty())));
    assert!(assertions.iter().all(|assertion| assertion
        .get("event_id")
        .and_then(Value::as_str)
        .is_some_and(|event_id| !event_id.is_empty())));
    assert!(assertions.iter().all(|assertion| assertion
        .get("reads")
        .and_then(Value::as_array)
        .is_some_and(|reads| !reads.is_empty())));
    assert!(assertions.iter().any(|assertion| assertion
        .get("reads")
        .and_then(Value::as_array)
        .is_some_and(|reads| reads.iter().any(|read| {
            read.get("kind").and_then(Value::as_str) == Some("effect")
                && read.get("head").and_then(Value::as_str) == Some("kind agent.tell")
                && read.get("match_count").and_then(Value::as_u64) == Some(6)
                && read
                    .get("matches")
                    .and_then(Value::as_array)
                    .is_some_and(|matches| {
                        matches.len() == 6
                            && matches.iter().all(|matched| {
                                matched.get("prompt_content_type").and_then(Value::as_str)
                                    == Some("markdown")
                            })
                    })
        }))));
    assert!(assertions.iter().any(|assertion| assertion
        .get("reads")
        .and_then(Value::as_array)
        .is_some_and(|reads| reads.iter().any(|read| {
            read.get("kind").and_then(Value::as_str) == Some("fact")
                && read.get("head").and_then(Value::as_str) == Some("LanguageE2EResult")
                && read.get("match_count").and_then(Value::as_u64) == Some(2)
                && read
                    .get("matches")
                    .and_then(Value::as_array)
                    .is_some_and(|matches| {
                        matches
                            .iter()
                            .all(|matched| matched.get("id").and_then(Value::as_str).is_some())
                    })
        }))));
    let executable_spec = dev.get("executable_spec").expect("executable spec");
    assert_eq!(
        executable_spec.get("status").and_then(Value::as_str),
        Some("passed")
    );
    assert_eq!(
        executable_spec
            .get("summary")
            .and_then(|summary| summary.get("total"))
            .and_then(Value::as_u64),
        Some(6)
    );
    assert_eq!(
        executable_spec
            .get("summary")
            .and_then(|summary| summary.get("passed"))
            .and_then(Value::as_u64),
        Some(6)
    );
    let acceptance_group = executable_spec
        .get("tags")
        .and_then(Value::as_array)
        .expect("executable spec tags")
        .iter()
        .find(|group| group.get("tag").and_then(Value::as_str) == Some("acceptance"))
        .expect("acceptance executable spec group");
    assert_eq!(
        acceptance_group
            .get("summary")
            .and_then(|summary| summary.get("total"))
            .and_then(Value::as_u64),
        Some(6)
    );
    assert_eq!(
        acceptance_group
            .get("assertions")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(6)
    );
    assert!(acceptance_group
        .get("assertions")
        .and_then(Value::as_array)
        .expect("acceptance assertions")
        .iter()
        .all(|assertion| assertion
            .get("description")
            .and_then(Value::as_str)
            .is_some_and(|description| !description.is_empty())));
    assert!(acceptance_group
        .get("assertions")
        .and_then(Value::as_array)
        .expect("acceptance assertions")
        .iter()
        .all(|assertion| assertion
            .get("event_id")
            .and_then(Value::as_str)
            .is_some_and(|event_id| !event_id.is_empty())));
    assert!(acceptance_group
        .get("assertions")
        .and_then(Value::as_array)
        .expect("acceptance assertions")
        .iter()
        .all(|assertion| assertion
            .get("reads")
            .and_then(Value::as_array)
            .is_some_and(|reads| !reads.is_empty())));
    let source_metadata = dev.get("source_metadata").expect("source metadata");
    assert!(source_metadata
        .get("targets")
        .and_then(Value::as_object)
        .expect("metadata targets")
        .contains_key("workflow:ProviderLanguageE2E"));
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("agent.turn.completed"))
            .count(),
        6
    );
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("baml.coerce.succeeded"))
            .count(),
        6
    );
    let result_languages = facts
        .iter()
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("LanguageE2EResult"))
        .map(|fact| {
            fact.get("value")
                .and_then(|value| value.get("language"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        result_languages,
        ["Arabic", "French", "German", "Hindi", "Japanese", "Spanish"]
            .into_iter()
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>()
    );
    let result_providers = facts
        .iter()
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("LanguageE2EResult"))
        .map(|fact| {
            fact.get("value")
                .and_then(|value| value.get("provider"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        result_providers
            .iter()
            .filter(|provider| provider.as_str() == "codex")
            .count(),
        2
    );
    assert_eq!(
        result_providers
            .iter()
            .filter(|provider| provider.as_str() == "claude")
            .count(),
        2
    );
    assert_eq!(
        result_providers
            .iter()
            .filter(|provider| provider.as_str() == "pi")
            .count(),
        2
    );

    let _ = fs::remove_file(store_path);
}

#[test]
fn dev_native_provider_records_policy_denial_from_source_required_capabilities() {
    let bin = env!("CARGO_BIN_EXE_whip");
    for provider in ["codex", "claude", "pi"] {
        let source_path = temp_workflow_path(&format!("native-policy-denial-e2e-{provider}"));
        fs::write(
            &source_path,
            r#"
workflow NativePolicyDenialE2E

agent worker {
  provider __PROVIDER__
  profile "repo-writer"
  capacity 1
  capabilities ["agent.tell", "repo.write"]
}

rule start_denied_work
  when started
  when worker is available
=> {
  tell worker requires ["repo.write"] "write in read-only native workflow"
}
"#
            .replace("__PROVIDER__", provider),
        )
        .expect("write native policy denial workflow");
        let store_path = temp_store_path();
        let dev = run_json(
            bin,
            &[
                "--store",
                store_path.to_str().expect("utf-8 temp path"),
                "--json",
                "dev",
                source_path.to_str().expect("utf-8 source path"),
                "--provider",
                provider,
                "--until",
                "idle",
            ],
        );
        let instance_id = dev
            .get("instance_id")
            .and_then(Value::as_str)
            .expect("instance id");
        let runs = run_json(
            bin,
            &[
                "--store",
                store_path.to_str().expect("utf-8 temp path"),
                "--json",
                "runs",
                instance_id,
            ],
        );
        let run = runs
            .as_array()
            .expect("runs array")
            .iter()
            .find(|run| run.get("provider").and_then(Value::as_str) == Some(provider))
            .expect("provider run");
        assert_eq!(run.get("status").and_then(Value::as_str), Some("failed"));
        assert_eq!(
            run.pointer("/native_lifecycle/status")
                .and_then(Value::as_str),
            Some("failed")
        );

        let log = run_json(
            bin,
            &[
                "--store",
                store_path.to_str().expect("utf-8 temp path"),
                "--json",
                "log",
                instance_id,
            ],
        );
        let events = log.as_array().expect("event array");
        assert!(
            events.iter().any(|event| {
                let provider_event_type = event
                    .get("payload")
                    .and_then(|payload| payload.get("provider_event_type"))
                    .and_then(Value::as_str);
                event.get("event_type").and_then(Value::as_str) == Some("agent.turn.failed")
                    && matches!(
                        provider_event_type,
                        Some("whip.native.boundary_error.workspace_denied")
                            | Some("whip.native.boundary_error.provider_health_unavailable")
                    )
            }),
            "expected native boundary failure event for {provider}: {events:#?}"
        );
        let _ = fs::remove_file(store_path);
        let _ = fs::remove_file(source_path);
    }
}

#[test]
fn dev_incident_router_routes_with_agentref_metadata() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("incident-router.whip");
    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            example.to_str().expect("utf-8 example path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let assertions = dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions");
    assert_eq!(assertions.len(), 4);
    assert!(assertions
        .iter()
        .all(|assertion| assertion.get("passed").and_then(Value::as_bool) == Some(true)));
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    let providers = facts
        .iter()
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("RoutedIncident"))
        .map(|fact| {
            fact.get("value")
                .and_then(|value| value.get("provider"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        providers,
        ["codex", "pi"]
            .into_iter()
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>()
    );

    let _ = fs::remove_file(store_path);
}

#[test]
fn dev_evaluates_case_branches_for_literal_and_optional_patterns() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("case-routing");
    fs::write(
        &source_path,
        r#"
workflow CaseRouting

class Task {
  provider "codex" | "claude"
  assignee string?
}

class Routed {
  provider string
  target string
  owner string
}

assert count(Routed where target == "codex") == 1
assert count(Routed where owner == "Ada") == 1

rule seed
  when started
=> {
  record Task {
    provider "codex"
    assignee "Ada"
  }
}

rule route
  when Task as task
=> {
  case task.provider {
    "codex" where task.assignee == null => {
      record Routed {
        provider task.provider
        target "wrong"
        owner "wrong"
      }
    }
    "codex" where task.assignee == "Ada" => {
      case task.assignee {
        Some owner => {
          record Routed {
            provider task.provider
            target "codex"
            owner owner
          }
        }
        None => {
          record Routed {
            provider task.provider
            target "codex"
            owner "unassigned"
          }
        }
      }
    }
    "claude" => {
      record Routed {
        provider task.provider
        target "claude"
        owner "unassigned"
      }
    }
    _ => {
      record Routed {
        provider task.provider
        target "unexpected"
        owner "unassigned"
      }
    }
  }
}
"#,
    )
    .expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    assert!(dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions")
        .iter()
        .all(|assertion| assertion.get("passed").and_then(Value::as_bool) == Some(true)));

    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let routed = facts
        .as_array()
        .expect("facts array")
        .iter()
        .find(|fact| fact.get("name").and_then(Value::as_str) == Some("Routed"))
        .expect("routed fact");
    assert_eq!(
        routed
            .get("value")
            .and_then(|value| value.get("owner"))
            .and_then(Value::as_str),
        Some("Ada")
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_does_not_leak_failed_case_branch_bindings() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("case-binding-leak");
    fs::write(
        &source_path,
        r#"
workflow CaseBindingLeak

class Task {
  assignee string?
}

class Routed {
  owner string
}

assert count(Routed where owner == "owner") == 1

rule seed
  when started
=> {
  record Task {
    assignee "Ada"
  }
}

rule route
  when Task as task
=> {
  case task.assignee {
    Some owner where false => {
      record Routed {
        owner "wrong"
      }
    }
    _ => {
      record Routed {
        owner owner
      }
    }
  }
}
"#,
    )
    .expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    assert!(dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions")
        .iter()
        .all(|assertion| assertion.get("passed").and_then(Value::as_bool) == Some(true)));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

fn run_terminal_branch_workflow(
    bin: &str,
    flag: Option<&str>,
    expected_branch: &str,
    expected_detail: &str,
) {
    let store_path = temp_store_path();
    let source_path = temp_workflow_path(&format!("terminal-{expected_branch}-branch"));
    fs::write(
        &source_path,
        r#"
workflow TerminalBranch

class WorkItem {
  title string
  body string
}

class MessageClassification {
  priority string
  summary string
  confidence float
}

class TerminalRoute {
  branch string
  detail string
}

class BranchEffect {
  branch string
}

coerce classifyMessage(title string, body string) -> MessageClassification {
  prompt "Classify"
}

rule seed
  when started
=> {
  record WorkItem {
    title "One"
    body "Two"
  }
}

rule classify_request
  when WorkItem as request
=> {
  coerce classifyMessage(request.title, request.body) as classification

  after classification completes {
    case classification {
      Completed result => {
        record TerminalRoute {
          branch "completed"
          detail result.summary
        }
        askHuman "completed branch effect"
      }
      Failed failure => {
        record TerminalRoute {
          branch "failed"
          detail failure.reason
        }
        askHuman "failed branch effect"
      }
      TimedOut timeout => {
        record TerminalRoute {
          branch "timed_out"
          detail timeout.summary
        }
        askHuman "timed_out branch effect"
      }
      Cancelled cancel => {
        record TerminalRoute {
          branch "cancelled"
          detail cancel.summary
        }
        askHuman "cancelled branch effect"
      }
    }
  }
}
"#,
    )
    .expect("write source");

    let mut args = vec![
        "--store",
        store_path.to_str().expect("utf-8 temp path"),
        "--json",
        "dev",
        source_path.to_str().expect("utf-8 source path"),
        "--provider",
        "fixture",
    ];
    if let Some(flag) = flag {
        args.push(flag);
    }
    args.extend(["--until", "idle"]);

    let dev = run_json(bin, &args);
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");
    let terminal_routes = facts
        .iter()
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("TerminalRoute"))
        .collect::<Vec<_>>();
    assert_eq!(terminal_routes.len(), 1, "{facts:#?}");
    let route = terminal_routes[0]
        .get("value")
        .and_then(Value::as_object)
        .expect("route value");
    assert_eq!(
        route.get("branch").and_then(Value::as_str),
        Some(expected_branch)
    );
    assert_eq!(
        route.get("detail").and_then(Value::as_str),
        Some(expected_detail)
    );
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("human.ask.created"))
            .count(),
        1,
        "{facts:#?}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_branches_on_all_terminal_union_payloads_and_branch_local_effects() {
    let bin = env!("CARGO_BIN_EXE_whip");
    run_terminal_branch_workflow(bin, None, "completed", "Fixture classification");
    run_terminal_branch_workflow(bin, Some("--fail"), "failed", "coerce failed");
    run_terminal_branch_workflow(bin, Some("--timeout"), "timed_out", "coerce timed out");
    run_terminal_branch_workflow(
        bin,
        Some("--cancel"),
        "cancelled",
        "fixture coerce cancelled",
    );
}

#[test]
fn dev_evaluates_shared_expression_kernel_for_guards_and_assertions() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("expression-kernel");
    fs::write(
        &source_path,
        r#"
workflow ExpressionKernelE2E

class ExprTask {
  provider "codex" | "claude" | "pi"
  priority int
  status "queued" | "blocked"
}

class ExprResult {
  provider string
  priority int
  status "accepted"
}

assert count(ExprResult) == 1
assert exists(ExprResult where provider == codex && priority >= 3)
assert count(ExprResult where provider == pi) == 0
assert count(ExprResult where priority > 1 && provider in ["codex", "claude"]) == 1
assert ("codex" in ["codex", "claude"]) && !("pi" in ["codex"])
assert count([]) == 0

rule seed
  when started
=> {
  record ExprTask {
    provider "codex"
    priority 5
    status "queued"
  }

  record ExprTask {
    provider "pi"
    priority 1
    status "blocked"
  }
}

rule accept_task
  when ExprTask as task where (task.priority >= 3 && task.provider in ["codex", "claude"]) && !(task.status == "blocked")
=> {
  record ExprResult {
    provider task.provider
    priority task.priority
    status "accepted"
  }
}
"#,
    )
    .expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    assert!(dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions")
        .iter()
        .all(|assertion| assertion.get("passed").and_then(Value::as_bool) == Some(true)));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn check_accepts_duration_and_time_ordering() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let source_path = temp_workflow_path("duration-time-ordering-check");
    fs::write(
        &source_path,
        r#"
@service
workflow DurationTimeOrderingCheck

class Window {
  elapsed duration
  limit duration
  opened_at time
  due_at time
}

@external
rule duration_guard
  when Window as window where window.elapsed < window.limit
=> {
}

@external
rule time_guard
  when Window as window where window.opened_at < window.due_at
=> {
}
"#,
    )
    .expect("write source");

    let output = Command::new(bin)
        .args(["check", source_path.to_str().expect("utf-8 source path")])
        .output()
        .expect("command runs");
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_seeds_duration_and_time_values_for_ordering() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("duration-time-ordering-literals");
    fs::write(
        &source_path,
        r#"
workflow DurationTimeOrderingLiterals

class Window {
  elapsed duration
  limit duration
  opened_at time
  due_at time
}

assert exists(Window where elapsed < limit)
assert exists(Window where opened_at < due_at)
assert count(Window where elapsed <= limit && due_at > opened_at) == 1

rule seed
  when started
=> {
  record Window {
    elapsed "PT1H"
    limit "PT2H"
    opened_at "2026-05-29T10:00:00.250-04:00"
    due_at "2026-05-29T14:00:00.500Z"
  }
}
"#,
    )
    .expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    assert!(dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions")
        .iter()
        .all(|assertion| assertion.get("passed").and_then(Value::as_bool) == Some(true)));
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    assert!(facts.as_array().expect("facts").iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("Window")
            && fact
                .get("value")
                .and_then(|value| value.get("elapsed"))
                .and_then(Value::as_str)
                == Some("PT1H")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn step_reports_typed_errors_for_invalid_external_duration_values() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("duration-time-external-invalid");
    fs::write(
        &source_path,
        r#"
workflow DurationTimeExternalInvalid

class Window {
  elapsed duration
  limit duration
}

class Outcome {
  status string
}

rule accept
  when Window as window where window.elapsed < window.limit
=> {
  record Outcome {
    status "accepted"
  }
}
"#,
    )
    .expect("write source");

    let started = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "run",
            source_path.to_str().expect("utf-8 source path"),
            "--json",
        ],
    );
    let instance_id = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();
    let mut store = SqliteStore::open(&store_path).expect("open store");
    let fact_value = r#"{"elapsed":"not-a-duration","limit":"PT1H"}"#;
    let fact = NewFact {
        fact_id: "external-window-invalid-duration",
        name: "Window",
        key: "external-window-invalid-duration",
        value_json: fact_value,
        schema_id: Some("Window"),
        provenance_class: "external",
        correlation_id: None,
        source_span_json: None,
    };
    store
        .commit_rule(RuleCommit {
            instance_id: &instance_id,
            rule: "external",
            trigger_event_id: None,
            facts: &[fact],
            consumed_fact_ids: &[],
            effects: &[],
            dependencies: &[],
            terminal: None,
            idempotency_key: Some("external-window-invalid-duration"),
        })
        .expect("commit external fact");

    let step = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "step",
            &instance_id,
            "--program",
            source_path.to_str().expect("utf-8 source path"),
        ],
    );
    let guards = step
        .get("guards")
        .and_then(Value::as_array)
        .expect("guards");
    assert!(guards.iter().any(|guard| {
        guard.get("status").and_then(Value::as_str) == Some("error")
            && guard
                .get("error")
                .and_then(Value::as_str)
                .is_some_and(|error| error.contains("invalid duration value `not-a-duration`"))
    }));
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            &instance_id,
        ],
    );
    assert!(!facts
        .as_array()
        .expect("facts")
        .iter()
        .any(|fact| { fact.get("name").and_then(Value::as_str) == Some("Outcome") }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn check_rejects_invalid_duration_and_time_literals() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let source_path = temp_workflow_path("duration-time-invalid-literals");
    fs::write(
        &source_path,
        r#"
workflow DurationTimeInvalidLiterals

class Window {
  elapsed duration
  opened_at time
}

rule seed
  when started
=> {
  record Window {
    elapsed "one hour"
    opened_at "noon"
  }
}
"#,
    )
    .expect("write source");

    let output = Command::new(bin)
        .args(["check", source_path.to_str().expect("utf-8 source path")])
        .output()
        .expect("command runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("field `Window.elapsed` has invalid duration literal"));
    assert!(stderr.contains("field `Window.opened_at` has invalid time literal"));

    let _ = fs::remove_file(source_path);
}

#[test]
fn check_rejects_bad_effect_payload_arguments() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let source_path = example_path("invalid/bad-effect-payload.whip");

    let output = Command::new(bin)
        .args(["check", source_path.to_str().expect("utf-8 source path")])
        .output()
        .expect("command runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("object literal without an expected object"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("class `Owner` has no field `handle`"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("field `coerce `reviewPayload`.metadata` expects `string`"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("field `coerce `reviewPayload`.score` expects `int`"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("field `loft claim.issue` receives incompatible expression type"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn check_rejects_bad_finite_domain_expressions() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let source_path = example_path("invalid/bad-finite-domain.whip");

    let output = Command::new(bin)
        .args(["check", source_path.to_str().expect("utf-8 source path")])
        .output()
        .expect("command runs");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("finite-domain value to unknown `pi`"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("finite-domain value to unknown `Missing`"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("finite-domain value to unknown `bad`"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("statically unsatisfiable finite-domain equality"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("statically unsatisfiable finite-domain exclusion"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn dev_reports_false_guards_without_committing_effects() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("false-guard");
    fs::write(
        &source_path,
        r#"
workflow FalseGuard

class Task {
  status "blocked"
}

class Result {
  status "accepted"
}

assert count(Result) == 0

rule seed
  when started
=> {
  record Task {
    status "blocked"
  }
}

rule accept
  when Task as task where task.status == "queued"
=> {
  record Result {
    status "accepted"
  }
}
"#,
    )
    .expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let guards = dev
        .get("steps")
        .and_then(Value::as_array)
        .expect("steps")
        .iter()
        .flat_map(|step| {
            step.get("guards")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .collect::<Vec<_>>();
    assert!(guards.iter().any(|guard| {
        guard.get("rule").and_then(Value::as_str) == Some("accept")
            && guard.get("status").and_then(Value::as_str) == Some("false")
            && guard.get("matched").and_then(Value::as_bool) == Some(false)
    }));

    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    assert_eq!(
        facts
            .as_array()
            .expect("facts")
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("Result"))
            .count(),
        0
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn check_reports_invalid_query_guards_before_dev_runs() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("guard-error");
    fs::write(
        &source_path,
        r#"
workflow GuardError

class Task {
  status "queued"
}

class Result {
  status "accepted"
}

rule seed
  when started
=> {
  record Task {
    status "queued"
  }
}

rule accept
  when Task as task where exists(Task where missing)
=> {
  record Result {
    status "accepted"
  }
}
"#,
    )
    .expect("write source");

    let output = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ])
        .output()
        .expect("command runs");
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "static diagnostics should not emit dev JSON\nstdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("rule `accept` fact query `Task` has non-boolean `where` expression"),
        "stderr:\n{stderr}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_evaluates_map_index_expressions() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("map-index");
    fs::write(
        &source_path,
        r#"
workflow MapIndex

class MapTask {
  metadata map<string>
}

class MapResult {
  priority string
}

assert exists(MapTask where metadata["priority"] == "high")
assert exists(MapTask where "priority" in metadata)
assert exists(MapTask where "missing" not in metadata)
assert count(MapResult where priority == "high") == 1

rule seed
  when started
=> {
  record MapTask {
    metadata { priority "high", owner "ada" }
  }
}

rule route
  when MapTask as task where "priority" in task.metadata && task.metadata["priority"] == "high"
=> {
  record MapResult {
    priority "high"
  }
}
"#,
    )
    .expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    assert!(dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions")
        .iter()
        .all(|assertion| assertion.get("passed").and_then(Value::as_bool) == Some(true)));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_distinguishes_missing_from_null_in_expression_kernel() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("missing-null");
    fs::write(
        &source_path,
        r#"
workflow MissingNull

class MaybeOwner {
  owner string?
  metadata map<string>
  status "open"
}

assert count(MaybeOwner) == 1
assert exists(MaybeOwner where owner == null)
assert count(MaybeOwner where metadata["missing"] == null) == 0
assert count(MaybeOwner where exists metadata["missing"]) == 0

rule seed
  when started
=> {
  record MaybeOwner {
    owner null
    metadata { present "value" }
    status "open"
  }
}
"#,
    )
    .expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    assert!(dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions")
        .iter()
        .all(|assertion| assertion.get("passed").and_then(Value::as_bool) == Some(true)));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_materializes_multiline_object_literals_and_coerce_object_args() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("object-literal-e2e");
    fs::write(
        &source_path,
        r#"
workflow ObjectLiteralE2E

class Owner {
  name string
}

class Payload {
  title string
  owner Owner
  metadata map<string>
  tags string[]
}

class Task {
  title string
  owner string
  payload Payload
  metadata map<string>
}

class Review {
  accepted bool
}

coerce reviewPayload(payload Payload, metadata map<string>) -> Review {
  Return whether the payload is valid.
}

rule seed
  when started
=> {
  record Task {
    title "Implement object literals"
    owner "Ada"
    payload {
      title "Implement object literals"
      owner {
        name "Ada"
      }
      metadata {
        phase "kernel"
        owner "Ada"
      }
      tags ["object", "effect"]
    }
    metadata {
      phase "kernel"
      owner "Ada"
    }
  }
}

rule review
  when Task as task where task.payload.owner.name == "Ada"
=> {
  coerce reviewPayload(
    {
      title task.title
      owner { name task.owner }
      metadata { phase task.metadata["phase"] owner task.owner }
      tags ["object", task.metadata["phase"]]
    },
    { phase task.metadata["phase"], owner task.owner }
  ) as review
}
"#,
    )
    .expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let task = facts
        .as_array()
        .expect("facts array")
        .iter()
        .find(|fact| fact.get("name").and_then(Value::as_str) == Some("Task"))
        .and_then(|fact| fact.get("value"))
        .expect("Task fact value");
    assert_eq!(
        task.pointer("/payload/owner/name").and_then(Value::as_str),
        Some("Ada")
    );
    assert_eq!(
        task.pointer("/payload/metadata/phase")
            .and_then(Value::as_str),
        Some("kernel")
    );

    let evidence = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "evidence",
            instance_id,
        ],
    );
    let arguments = evidence
        .get("evidence")
        .and_then(Value::as_array)
        .expect("evidence array")
        .iter()
        .find(|item| item.get("kind").and_then(Value::as_str) == Some("baml.coerce.provider"))
        .and_then(|item| item.get("metadata"))
        .and_then(|metadata| metadata.get("arguments"))
        .expect("baml arguments");
    assert_eq!(
        arguments.get("redacted").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        arguments.pointer("/shape/type").and_then(Value::as_str),
        Some("object")
    );
    let arguments_json = arguments.to_string();
    assert!(!arguments_json.contains("Ada"));
    assert!(!arguments_json.contains("kernel"));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_reports_failed_assertions_with_nonzero_exit() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("assertion-failure");
    fs::write(
        &source_path,
        r#"
workflow AssertionFailure

class Seen {
  status "ok"
}

assert count(Seen) == 2

rule seed
  when started
=> {
  record Seen {
    status "ok"
  }
}
"#,
    )
    .expect("write source");

    let output = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ])
        .output()
        .expect("command runs");
    assert!(!output.status.success());
    let dev: Value = serde_json::from_slice(&output.stdout).expect("json stdout");
    let assertion = dev
        .get("assertions")
        .and_then(Value::as_array)
        .and_then(|assertions| assertions.first())
        .expect("assertion");
    assert_eq!(
        assertion.get("status").and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        assertion.get("passed").and_then(Value::as_bool),
        Some(false)
    );
    let assertion_event_id = assertion
        .get("event_id")
        .and_then(Value::as_str)
        .expect("assertion event id")
        .to_owned();
    let assertion_diagnostic_id = assertion
        .pointer("/diagnostic_ids/0")
        .and_then(Value::as_str)
        .expect("assertion diagnostic id")
        .to_owned();
    assert!(dev
        .get("diagnostics")
        .and_then(Value::as_array)
        .expect("dev report diagnostics")
        .iter()
        .any(
            |diagnostic| diagnostic.get("diagnostic_id").and_then(Value::as_str)
                == Some(assertion_diagnostic_id.as_str())
        ));
    assert_eq!(
        assertion
            .pointer("/expected/predicate")
            .and_then(Value::as_str),
        Some("==")
    );
    assert_eq!(
        assertion.pointer("/expected/left").and_then(Value::as_str),
        Some("count(Seen)")
    );
    assert_eq!(
        assertion.pointer("/expected/right").and_then(Value::as_str),
        Some("2")
    );
    assert_eq!(
        assertion
            .pointer("/actual_values/left")
            .and_then(Value::as_i64),
        Some(1)
    );
    assert_eq!(
        assertion
            .pointer("/actual_values/right")
            .and_then(Value::as_i64),
        Some(2)
    );
    assert_eq!(
        assertion
            .pointer("/actual_values/result")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        assertion.get("failure_reason").and_then(Value::as_str),
        Some("predicate `==` evaluated to false")
    );
    assert_eq!(
        assertion.pointer("/reads/0/kind").and_then(Value::as_str),
        Some("fact")
    );
    assert_eq!(
        assertion.pointer("/reads/0/head").and_then(Value::as_str),
        Some("Seen")
    );
    assert_eq!(
        assertion
            .pointer("/reads/0/match_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        assertion
            .pointer("/reads/0/matches/0/name")
            .and_then(Value::as_str),
        Some("Seen")
    );
    let executable_spec = dev.get("executable_spec").expect("executable spec");
    assert_eq!(
        executable_spec.get("status").and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        executable_spec
            .pointer("/summary/total")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        executable_spec
            .pointer("/summary/failed")
            .and_then(Value::as_u64),
        Some(1)
    );
    let untagged = executable_spec.get("untagged").expect("untagged group");
    assert_eq!(
        untagged.get("status").and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        untagged
            .pointer("/assertions/0/status")
            .and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        untagged
            .pointer("/assertions/0/event_id")
            .and_then(Value::as_str),
        Some(assertion_event_id.as_str())
    );
    assert_eq!(
        untagged
            .pointer("/assertions/0/diagnostic_ids/0")
            .and_then(Value::as_str),
        Some(assertion_diagnostic_id.as_str())
    );
    assert_eq!(
        untagged
            .pointer("/assertions/0/reads/0/source")
            .and_then(Value::as_str),
        Some("fact:Seen")
    );

    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    assert_eq!(
        facts
            .as_array()
            .expect("facts")
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("Seen"))
            .count(),
        1
    );
    let diagnostics = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "diagnostics",
            instance_id,
        ],
    );
    let diagnostics = diagnostics.as_array().expect("diagnostics array");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.get("diagnostic_id").and_then(Value::as_str)
            == Some(assertion_diagnostic_id.as_str())
            && diagnostic.get("code").and_then(Value::as_str) == Some("assertion.failed")
            && diagnostic.get("subject_type").and_then(Value::as_str) == Some("assertion")
            && diagnostic.get("event_id").and_then(Value::as_str).is_some()
            && diagnostic.get("event_id").and_then(Value::as_str)
                == Some(assertion_event_id.as_str())
            && diagnostic
                .get("assertion_id")
                .and_then(Value::as_str)
                .is_some()
            && diagnostic
                .pointer("/source_span/construct")
                .and_then(Value::as_str)
                == Some("assertion")
            && diagnostic
                .pointer("/source_span/path")
                .and_then(Value::as_str)
                == source_path.to_str()
            && diagnostic
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains("count(Seen) == 2"))
    }));
    let log = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            instance_id,
        ],
    );
    assert!(log.as_array().expect("events").iter().any(|event| {
        event.get("event_type").and_then(Value::as_str) == Some("assertion.failed")
            && event.get("event_id").and_then(Value::as_str) == Some(assertion_event_id.as_str())
            && event.pointer("/payload/result").and_then(Value::as_str) == Some("fail")
            && event
                .pointer("/payload/assertion_text")
                .and_then(Value::as_str)
                == Some("count(Seen) == 2")
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_streams_ndjson_progress_and_final_report() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("dev-stream");
    fs::write(
        &source_path,
        r#"
workflow DevStream

class Seen {
  status "ok"
}

assert count(Seen) == 1

rule seed
  when started
=> {
  record Seen {
    status "ok"
  }
}
"#,
    )
    .expect("write source");

    let output = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
            "--stream",
            "ndjson",
        ])
        .output()
        .expect("command runs");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    assert!(!stdout.contains("\ndev inst_"), "{stdout}");
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("ndjson line"))
        .collect::<Vec<_>>();
    assert!(events.len() >= 6, "{stdout}");
    assert!(events
        .iter()
        .all(|event| event.get("schema").and_then(Value::as_str)
            == Some("whipplescript.dev_stream.v0")));
    assert_eq!(
        events
            .iter()
            .enumerate()
            .map(|(index, event)| (index, event.get("sequence").and_then(Value::as_u64)))
            .collect::<Vec<_>>(),
        events
            .iter()
            .enumerate()
            .map(|(index, _)| (index, Some(index as u64)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        events
            .first()
            .and_then(|event| event.get("event"))
            .and_then(Value::as_str),
        Some("dev.started")
    );
    assert!(events
        .iter()
        .any(|event| event.get("event").and_then(Value::as_str) == Some("dev.step")));
    let event_batches = events
        .iter()
        .filter(|event| event.get("event").and_then(Value::as_str) == Some("dev.events"))
        .collect::<Vec<_>>();
    assert!(!event_batches.is_empty(), "{stdout}");
    let raw_event_sequences = event_batches
        .iter()
        .flat_map(|batch| {
            batch
                .pointer("/data/events")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|event| event.get("sequence").and_then(Value::as_i64))
        .collect::<Vec<_>>();
    assert!(!raw_event_sequences.is_empty(), "{stdout}");
    assert_eq!(
        raw_event_sequences,
        raw_event_sequences
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    );
    for batch in event_batches {
        let count = batch.pointer("/data/count").and_then(Value::as_u64);
        let events = batch
            .pointer("/data/events")
            .and_then(Value::as_array)
            .expect("dev.events data.events");
        assert_eq!(count, Some(events.len() as u64));
    }
    assert!(events
        .iter()
        .any(|event| event.get("event").and_then(Value::as_str) == Some("dev.worker")));
    assert!(events
        .iter()
        .any(|event| event.get("event").and_then(Value::as_str) == Some("dev.idle")));
    let assertion_event = events
        .iter()
        .find(|event| event.get("event").and_then(Value::as_str) == Some("dev.assertions"))
        .expect("dev.assertions event");
    assert_eq!(
        assertion_event
            .pointer("/data/status")
            .and_then(Value::as_str),
        Some("passed")
    );
    assert_eq!(
        assertion_event
            .pointer("/data/summary/total")
            .and_then(Value::as_u64),
        Some(1)
    );
    let report = events
        .last()
        .filter(|event| event.get("event").and_then(Value::as_str) == Some("dev.report"))
        .and_then(|event| event.get("data"))
        .expect("final report");
    assert_eq!(
        report.get("schema").and_then(Value::as_str),
        Some("whipplescript.dev_report.v0")
    );
    assert_eq!(
        report
            .pointer("/executable_spec/status")
            .and_then(Value::as_str),
        Some("passed")
    );
    assert_eq!(
        report
            .get("assertions")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn accept_runs_json_fixture_through_dev_report_contract() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("accept-fixture-workflow");
    let fixture_path = temp_workflow_path("accept-fixture").with_extension("json");
    fs::write(
        &source_path,
        r#"
@fixture
@acceptance
description "Fixture-backed acceptance workflow"
workflow AcceptFixture

class Seen {
  status "ok"
}

@acceptance
assert count(Seen) == 1

rule seed
  when started
=> {
  record Seen {
    status "ok"
  }
}
"#,
    )
    .expect("write source");
    fs::write(
        &fixture_path,
        json!({
            "schema": "whipplescript.acceptance_fixture.v0",
            "workflow": source_path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("workflow filename"),
            "provider": "fixture",
            "actions": [
                {"type": "pause", "reason": "exercise fixture control-plane action"},
                {"type": "resume"}
            ],
            "include_tags": ["acceptance"],
            "expect": {
                "dev_status": "success",
                "workflow": "AcceptFixture",
                "status": "passed",
                "source_metadata": {
                    "targets": [
                        {
                            "target_kind": "workflow",
                            "target": "AcceptFixture",
                            "tags": ["fixture", "acceptance"],
                            "description": "Fixture-backed acceptance workflow"
                        }
                    ]
                },
                "diagnostics": 0,
                "actions": [
                    {"type": "pause", "count": 1},
                    {"type": "resume", "count": 1}
                ],
                "trace": {
                    "conformance": {"ok": true},
                    "groups": [
                        {"type": "instance_paused", "count": 1},
                        {"type": "instance_resumed", "count": 1}
                    ]
                },
                "assertions": {
                    "total": 1,
                    "passed": 1,
                    "failed": 0,
                    "error": 0
                },
                "assertion_tags": [
                    {"tag": "acceptance", "total": 1, "passed": 1, "failed": 0, "error": 0}
                ],
                "summary": {
                    "facts": 1,
                    "effects": 0
                },
                "facts": [
                    {"name": "Seen", "count": 1}
                ],
                "runs": [
                    {"provider": "fixture", "status": "completed", "count": 0, "artifact_count": 0}
                ],
                "artifacts": [
                    {"kind": "transcript", "count": 0}
                ]
            }
        })
        .to_string(),
    )
    .expect("write fixture");

    let report = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "accept",
            fixture_path.to_str().expect("utf-8 fixture path"),
        ],
    );

    assert_eq!(
        report.get("schema").and_then(Value::as_str),
        Some("whipplescript.acceptance_report.v0")
    );
    assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
    assert_eq!(
        report
            .get("failures")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        report
            .pointer("/dev_report/workflow")
            .and_then(Value::as_str),
        Some("AcceptFixture")
    );
    assert_eq!(
        report
            .pointer("/dev_report/assertion_filter/selected")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/dev_report/executable_spec/tags/0/tag")
            .and_then(Value::as_str),
        Some("acceptance")
    );
    assert_eq!(
        report
            .pointer("/dev_report/executable_spec/tags/0/summary/total")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/facts/0/name")
            .and_then(Value::as_str),
        Some("Seen")
    );
    assert_eq!(
        report
            .pointer("/observed/summary/facts")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/summary/effects")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        report
            .pointer("/observed/facts/0/count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/executable_spec/status")
            .and_then(Value::as_str),
        Some("passed")
    );
    assert_eq!(
        report
            .pointer("/observed/executable_spec/summary/total")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/executable_spec/tags/0/tag")
            .and_then(Value::as_str),
        Some("acceptance")
    );
    assert_eq!(
        report
            .pointer("/observed/diagnostics_by_code")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        report
            .pointer("/observed/assertion_reads/0/source")
            .and_then(Value::as_str),
        Some("fact:Seen")
    );
    assert_eq!(
        report
            .pointer("/observed/assertion_reads/0/match_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/assertion_reads/0/matches/0/name")
            .and_then(Value::as_str),
        Some("Seen")
    );
    assert_eq!(
        report
            .pointer("/observed/assertion_reads/0/matches/0/count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/actions/0/type")
            .and_then(Value::as_str),
        Some("pause")
    );
    assert_eq!(
        report
            .pointer("/observed/actions/1/type")
            .and_then(Value::as_str),
        Some("resume")
    );
    assert_eq!(
        report
            .pointer("/observed/source_metadata/summary/targets")
            .and_then(Value::as_u64),
        Some(2)
    );
    let observed_targets = report
        .pointer("/observed/source_metadata/targets")
        .and_then(Value::as_array)
        .expect("observed source metadata targets");
    let workflow_target = observed_targets
        .iter()
        .find(|target| target.get("key").and_then(Value::as_str) == Some("workflow:AcceptFixture"))
        .expect("workflow source metadata target");
    assert_eq!(
        workflow_target.get("description").and_then(Value::as_str),
        Some("Fixture-backed acceptance workflow")
    );
    assert_eq!(
        report
            .pointer("/observed/runs/summary/total")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        report
            .pointer("/observed/runs/summary/artifact_count")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        report
            .pointer("/observed/artifacts/summary/total")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        report
            .pointer("/observed/trace/conformance/ok")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(report
        .pointer("/observed/trace/summary/events")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0));
    assert!(report
        .pointer("/observed/trace/summary/abstract_events")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0));
    let trace_items = report
        .pointer("/observed/trace/items")
        .and_then(Value::as_array)
        .expect("trace items");
    assert!(
        trace_items
            .iter()
            .any(|item| item.pointer("/event/type").and_then(Value::as_str)
                == Some("instance_paused"))
    );
    assert!(trace_items.iter().any(
        |item| item.pointer("/event/type").and_then(Value::as_str) == Some("instance_resumed")
    ));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(fixture_path);
}

#[test]
fn accept_fixture_input_seeds_workflow_start_facts() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("accept-fixture-input-workflow");
    let fixture_path = temp_workflow_path("accept-fixture-input").with_extension("json");
    fs::write(
        &source_path,
        r#"
workflow AcceptFixtureInput {
  input request Request
}

class Request {
  title "Review parser"
}

class SeenRequest {
  title "Review parser"
}

@acceptance
assert count(SeenRequest) == 1

rule seedFromInput
  when Request as request
=> {
  record SeenRequest {
    title request.title
  }
}
"#,
    )
    .expect("write source");
    fs::write(
        &fixture_path,
        json!({
            "schema": "whipplescript.acceptance_fixture.v0",
            "workflow": source_path,
            "provider": "fixture",
            "include_tags": ["acceptance"],
            "input": {
                "request": {
                    "title": "Review parser"
                }
            },
            "expect": {
                "dev_status": "success",
                "workflow": "AcceptFixtureInput",
                "status": "passed",
                "diagnostics": 0,
                "assertions": {
                    "total": 1,
                    "passed": 1,
                    "failed": 0,
                    "error": 0
                },
                "assertion_tags": [
                    {"tag": "acceptance", "total": 1, "passed": 1, "failed": 0, "error": 0}
                ],
                "summary": {
                    "facts": 2,
                    "effects": 0
                },
                "facts": [
                    {"name": "Request", "count": 1},
                    {"name": "SeenRequest", "count": 1}
                ]
            }
        })
        .to_string(),
    )
    .expect("write fixture");

    let report = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "accept",
            fixture_path.to_str().expect("utf-8 fixture path"),
        ],
    );

    assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
    assert_eq!(
        report
            .pointer("/observed/summary/facts")
            .and_then(Value::as_u64),
        Some(2)
    );
    let observed_facts = report
        .pointer("/observed/facts")
        .and_then(Value::as_array)
        .expect("observed facts");
    assert!(observed_facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("Request")
            && fact.get("count").and_then(Value::as_u64) == Some(1)
    }));
    assert!(observed_facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("SeenRequest")
            && fact.get("count").and_then(Value::as_u64) == Some(1)
    }));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(fixture_path);
}

#[test]
fn accept_fixture_setup_facts_seed_active_facts() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("accept-fixture-setup-facts-workflow");
    let fixture_path = temp_workflow_path("accept-fixture-setup-facts").with_extension("json");
    fs::write(
        &source_path,
        r#"
workflow AcceptFixtureSetupFacts

class ExternalTask {
  title string
  status "queued"
}

class SetupResult {
  title string
  status "done"
}

@acceptance
assert count(SetupResult) == 1

@acceptance
assert count(ExternalTask where status == "queued") == 0

rule handle_setup_fact
  when ExternalTask as task where task.status == "queued"
=> {
  done task

  record SetupResult {
    title task.title
    status "done"
  }
}
"#,
    )
    .expect("write source");
    fs::write(
        &fixture_path,
        json!({
            "schema": "whipplescript.acceptance_fixture.v0",
            "workflow": source_path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("workflow filename"),
            "provider": "fixture",
            "setup": {
                "facts": [
                    {
                        "name": "ExternalTask",
                        "value": {
                            "title": "Seeded from fixture setup",
                            "status": "queued"
                        }
                    }
                ]
            },
            "include_tags": ["acceptance"],
            "expect": {
                "dev_status": "success",
                "workflow": "AcceptFixtureSetupFacts",
                "status": "passed",
                "diagnostics": 0,
                "assertions": {
                    "total": 2,
                    "passed": 2,
                    "failed": 0,
                    "error": 0
                },
                "assertion_tags": [
                    {"tag": "acceptance", "total": 2, "passed": 2, "failed": 0, "error": 0}
                ],
                "assertion_reads": [
                    {
                        "source": "fact:SetupResult",
                        "match_count": 1,
                        "matches": [
                            {"name": "SetupResult", "provenance_class": "rule", "count": 1}
                        ]
                    },
                    {
                        "source": "fact:ExternalTask where status == \"queued\"",
                        "match_count": 0
                    }
                ],
                "summary": {
                    "facts": 1,
                    "effects": 0
                },
                "facts": [
                    {"name": "SetupResult", "count": 1}
                ],
                "runs": [
                    {"provider": "fixture", "status": "completed", "count": 0, "artifact_count": 0}
                ]
            }
        })
        .to_string(),
    )
    .expect("write fixture");

    let report = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "accept",
            fixture_path.to_str().expect("utf-8 fixture path"),
        ],
    );
    assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
    assert_eq!(
        report
            .pointer("/observed/facts/0/name")
            .and_then(Value::as_str),
        Some("SetupResult")
    );
    assert_eq!(
        report
            .pointer("/dev_report/steps/0/facts_consumed")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/dev_report/steps/0/facts_created")
            .and_then(Value::as_u64),
        Some(1)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(fixture_path);
}

#[test]
fn accept_fixture_cancel_action_records_trace() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("accept-fixture-cancel-workflow");
    let fixture_path = temp_workflow_path("accept-fixture-cancel").with_extension("json");
    fs::write(
        &source_path,
        r#"
workflow AcceptFixtureCancel

class Seen {
  status "ok"
}

rule seed
  when started
=> {
  record Seen {
    status "ok"
  }
}
"#,
    )
    .expect("write source");
    fs::write(
        &fixture_path,
        json!({
            "schema": "whipplescript.acceptance_fixture.v0",
            "workflow": source_path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("workflow filename"),
            "provider": "fixture",
            "actions": [
                {"type": "cancel", "reason": "exercise fixture control-plane cancel"}
            ],
            "expect": {
                "dev_status": "success",
                "workflow": "AcceptFixtureCancel",
                "status": "passed",
                "diagnostics": 0,
                "actions": [
                    {"type": "cancel", "count": 1}
                ],
                "trace": {
                    "conformance": {"ok": true},
                    "summary": {
                        "abstract_events": 1
                    },
                    "groups": [
                        {"type": "instance_cancelled", "count": 1}
                    ]
                },
                "assertions": {
                    "total": 0,
                    "passed": 0,
                    "failed": 0,
                    "error": 0
                },
                "summary": {
                    "facts": 0,
                    "effects": 0
                },
                "runs": [
                    {"provider": "fixture", "status": "completed", "count": 0, "artifact_count": 0}
                ]
            }
        })
        .to_string(),
    )
    .expect("write fixture");

    let report = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "accept",
            fixture_path.to_str().expect("utf-8 fixture path"),
        ],
    );
    assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
    assert_eq!(
        report
            .pointer("/observed/actions/0/type")
            .and_then(Value::as_str),
        Some("cancel")
    );
    assert_eq!(
        report
            .pointer("/observed/trace/groups/0/type")
            .and_then(Value::as_str),
        Some("instance_cancelled")
    );
    assert_eq!(
        report
            .pointer("/observed/summary/facts")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        report
            .pointer("/observed/summary/effects")
            .and_then(Value::as_u64),
        Some(0)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(fixture_path);
}

#[test]
fn accept_observes_provider_runs_and_artifacts() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("accept-fixture-native-provider-workflow");
    let fixture_path = temp_workflow_path("accept-fixture-native-provider").with_extension("json");
    fs::write(
        &source_path,
        r#"
workflow AcceptFixtureNativeProvider

agent worker {
  provider native-fixture
  profile "repo-writer"
  capacity 1
}

rule startNativeWork
  when started
  when worker is available
=> {
  tell worker "create native fixture evidence"
}
"#,
    )
    .expect("write source");
    fs::write(
        &fixture_path,
        json!({
            "schema": "whipplescript.acceptance_fixture.v0",
            "workflow": source_path,
            "provider": "native-fixture",
            "expect": {
                "dev_status": "success",
                "workflow": "AcceptFixtureNativeProvider",
                "status": "passed",
                "diagnostics": 0,
                "assertions": {
                    "total": 0,
                    "passed": 0,
                    "failed": 0,
                    "error": 0
                },
                "summary": {
                    "facts": 3,
                    "effects": 1
                },
                "facts": [
                    {"name": "agent.turn.started", "count": 1},
                    {"name": "agent.turn.artifact_captured", "count": 1},
                    {"name": "agent.turn.completed", "count": 1}
                ],
                "effects": [
                    {"kind": "agent.tell", "status": "completed", "count": 1}
                ],
                "runs": [
                    {"provider": "native-fixture", "status": "completed", "count": 1, "artifact_count": 1}
                ],
                "artifacts": [
                    {"kind": "transcript", "mime_type": "text/plain", "count": 1}
                ],
                "evidence": [
                    {"kind": "agent.turn.native_event", "subject_type": "run", "count": 3},
                    {"kind": "agent.turn.native_provider", "subject_type": "run", "count": 3},
                    {"kind": "skills.injected", "subject_type": "run", "count": 1},
                    {"kind": "rule.committed", "subject_type": "rule_commit", "count": 1}
                ]
            }
        })
        .to_string(),
    )
    .expect("write fixture");

    let report = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "accept",
            fixture_path.to_str().expect("utf-8 fixture path"),
        ],
    );

    assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
    assert_eq!(
        report
            .pointer("/observed/runs/summary/total")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/runs/summary/artifact_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/runs/groups/0/provider")
            .and_then(Value::as_str),
        Some("native-fixture")
    );
    assert_eq!(
        report
            .pointer("/observed/runs/groups/0/artifact_count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/artifacts/summary/total")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/artifacts/groups/0/kind")
            .and_then(Value::as_str),
        Some("transcript")
    );
    assert_eq!(
        report
            .pointer("/observed/artifacts/groups/0/mime_type")
            .and_then(Value::as_str),
        Some("text/plain")
    );
    let artifact_items = report
        .pointer("/observed/artifacts/items")
        .and_then(Value::as_array)
        .expect("observed artifact items");
    let transcript_artifact = artifact_items
        .iter()
        .find(|item| item.get("kind").and_then(Value::as_str) == Some("transcript"))
        .expect("transcript artifact item");
    assert!(transcript_artifact
        .get("artifact_id")
        .and_then(Value::as_str)
        .is_some_and(|artifact_id| !artifact_id.is_empty()));
    assert!(transcript_artifact
        .get("run_id")
        .and_then(Value::as_str)
        .is_some_and(|run_id| !run_id.is_empty()));
    assert_eq!(
        transcript_artifact.get("mime_type").and_then(Value::as_str),
        Some("text/plain")
    );
    assert_eq!(
        report
            .pointer("/observed/evidence/summary/total")
            .and_then(Value::as_u64),
        Some(8)
    );
    assert_eq!(
        report
            .pointer("/observed/evidence/groups/0/kind")
            .and_then(Value::as_str),
        Some("agent.turn.native_event")
    );
    let evidence_items = report
        .pointer("/observed/evidence/items")
        .and_then(Value::as_array)
        .expect("observed evidence items");
    let native_event_evidence = evidence_items
        .iter()
        .find(|item| item.get("kind").and_then(Value::as_str) == Some("agent.turn.native_event"))
        .expect("native event evidence item");
    assert_eq!(
        native_event_evidence
            .get("subject_type")
            .and_then(Value::as_str),
        Some("run")
    );
    assert!(native_event_evidence
        .get("subject_id")
        .and_then(Value::as_str)
        .is_some_and(|subject_id| !subject_id.is_empty()));
    assert!(native_event_evidence
        .get("summary")
        .and_then(Value::as_str)
        .is_some_and(|summary| !summary.is_empty()));
    let trace_items = report
        .pointer("/observed/trace/items")
        .and_then(Value::as_array)
        .expect("trace items");
    let run_started = trace_items
        .iter()
        .find(|item| item.pointer("/event/type").and_then(Value::as_str) == Some("run_started"))
        .expect("run_started trace item");
    assert!(run_started
        .pointer("/event/run_id")
        .and_then(Value::as_str)
        .is_some_and(|run_id| !run_id.is_empty()));
    assert!(run_started
        .pointer("/event/effect_id")
        .and_then(Value::as_str)
        .is_some_and(|effect_id| !effect_id.is_empty()));
    assert!(
        trace_items
            .iter()
            .any(|item| item.pointer("/event/type").and_then(Value::as_str)
                == Some("effect_terminal"))
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(fixture_path);
}

#[test]
fn accept_observes_human_inbox_requests() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("accept-fixture-human-inbox-workflow");
    let fixture_path = temp_workflow_path("accept-fixture-human-inbox").with_extension("json");
    fs::write(
        &source_path,
        r#"
workflow AcceptFixtureHumanInbox

@acceptance
assert count(effect kind human.ask where status == completed) == 1

rule ask
  when started
=> {
  askHuman """application/json
  {
    "question": "Approve this release?"
  }
  """
}
"#,
    )
    .expect("write source");
    fs::write(
        &fixture_path,
        json!({
            "schema": "whipplescript.acceptance_fixture.v0",
            "workflow": source_path,
            "provider": "fixture",
            "setup": {
                "inbox": [
                    {
                        "prompt": "Pre-existing release note review",
                        "severity": "urgent",
                        "choices": ["approve", "reject"],
                        "freeform_allowed": false
                    }
                ]
            },
            "include_tags": ["acceptance"],
            "expect": {
                "dev_status": "success",
                "workflow": "AcceptFixtureHumanInbox",
                "status": "passed",
                "diagnostics": 0,
                "assertions": {
                    "total": 1,
                    "passed": 1,
                    "failed": 0,
                    "error": 0
                },
                "effects": [
                    {"kind": "human.ask", "status": "completed", "count": 1}
                ],
                "inbox": [
                    {"status": "pending", "severity": "normal", "count": 1},
                    {"status": "pending", "severity": "urgent", "count": 1}
                ],
                "runs": [
                    {"provider": "fixture", "status": "completed", "count": 1}
                ]
            }
        })
        .to_string(),
    )
    .expect("write fixture");

    let report = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "accept",
            fixture_path.to_str().expect("utf-8 fixture path"),
        ],
    );

    assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
    assert_eq!(
        report
            .pointer("/observed/inbox/summary/total")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        report
            .pointer("/observed/inbox/groups/0/status")
            .and_then(Value::as_str),
        Some("pending")
    );
    assert_eq!(
        report
            .pointer("/observed/inbox/groups/0/severity")
            .and_then(Value::as_str),
        Some("normal")
    );
    assert_eq!(
        report
            .pointer("/observed/inbox/groups/0/count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/inbox/groups/1/status")
            .and_then(Value::as_str),
        Some("pending")
    );
    assert_eq!(
        report
            .pointer("/observed/inbox/groups/1/severity")
            .and_then(Value::as_str),
        Some("urgent")
    );
    assert_eq!(
        report
            .pointer("/observed/inbox/groups/1/count")
            .and_then(Value::as_u64),
        Some(1)
    );
    let assertion_match = report
        .pointer("/observed/assertion_reads/0/matches/0")
        .expect("human.ask assertion match");
    let trace_sequences = assertion_match
        .get("trace_sequences")
        .and_then(Value::as_array)
        .expect("trace sequences");
    let evidence_ids = assertion_match
        .get("evidence_ids")
        .and_then(Value::as_array)
        .expect("evidence ids");
    assert_eq!(
        assertion_match.get("trace_items").and_then(Value::as_u64),
        Some(trace_sequences.len() as u64)
    );
    assert_eq!(
        assertion_match
            .get("evidence_items")
            .and_then(Value::as_u64),
        Some(evidence_ids.len() as u64)
    );
    assert!(trace_sequences
        .iter()
        .all(|sequence| sequence.as_i64().is_some_and(|sequence| sequence > 0)));
    assert!(evidence_ids
        .iter()
        .all(|evidence_id| evidence_id.as_str().is_some_and(|id| !id.is_empty())));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(fixture_path);
}

#[test]
fn accept_rejects_invalid_setup_inbox_items() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("accept-fixture-invalid-inbox-workflow");
    let fixture_path = temp_workflow_path("accept-fixture-invalid-inbox").with_extension("json");
    fs::write(
        &source_path,
        r#"
workflow AcceptFixtureInvalidInbox

rule noop
  when started
=> {}
"#,
    )
    .expect("write source");
    fs::write(
        &fixture_path,
        json!({
            "schema": "whipplescript.acceptance_fixture.v0",
            "workflow": source_path,
            "setup": {
                "inbox": [
                    {
                        "prompt": "Review this before running",
                        "choices": {"approve": true}
                    }
                ]
            },
            "expect": {
                "dev_status": "success"
            }
        })
        .to_string(),
    )
    .expect("write fixture");

    let output = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "accept",
            fixture_path.to_str().expect("utf-8 fixture path"),
        ])
        .output()
        .expect("command runs");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("setup.inbox[0].choices must be an array"),
        "{stderr}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(fixture_path);
}

#[test]
fn accept_rejects_unsupported_setup_collections() {
    let bin = env!("CARGO_BIN_EXE_whip");
    for key in ["effects", "artifacts"] {
        let store_path = temp_store_path();
        let source_path = temp_workflow_path(&format!("accept-fixture-unsupported-setup-{key}"));
        let fixture_path = temp_workflow_path(&format!("accept-fixture-unsupported-setup-{key}"))
            .with_extension("json");
        fs::write(
            &source_path,
            r#"
workflow AcceptFixtureUnsupportedSetup

rule noop
  when started
=> {}
"#,
        )
        .expect("write source");
        let mut setup = serde_json::Map::new();
        setup.insert(key.to_owned(), json!([]));
        fs::write(
            &fixture_path,
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "setup": setup,
                "expect": {
                    "dev_status": "success"
                }
            })
            .to_string(),
        )
        .expect("write fixture");

        let output = Command::new(bin)
            .args([
                "--store",
                store_path.to_str().expect("utf-8 temp path"),
                "accept",
                fixture_path.to_str().expect("utf-8 fixture path"),
            ])
            .output()
            .expect("command runs");

        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!(
                "setup.{key} is not supported in acceptance_fixture.v0"
            )),
            "{stderr}"
        );

        let _ = fs::remove_file(store_path);
        let _ = fs::remove_file(source_path);
        let _ = fs::remove_file(fixture_path);
    }
}

#[test]
fn accept_rejects_zero_max_iterations() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("accept-fixture-zero-max-iterations-workflow");
    let fixture_path =
        temp_workflow_path("accept-fixture-zero-max-iterations").with_extension("json");
    fs::write(
        &source_path,
        r#"
workflow AcceptFixtureZeroMaxIterations

rule noop
  when started
=> {}
"#,
    )
    .expect("write source");
    fs::write(
        &fixture_path,
        json!({
            "schema": "whipplescript.acceptance_fixture.v0",
            "workflow": source_path,
            "max_iterations": 0,
            "expect": {
                "dev_status": "success"
            }
        })
        .to_string(),
    )
    .expect("write fixture");

    let output = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "accept",
            fixture_path.to_str().expect("utf-8 fixture path"),
        ])
        .output()
        .expect("command runs");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("`max_iterations` must be at least 1"),
        "{stderr}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(fixture_path);
}

#[test]
fn accept_rejects_invalid_fixture_shape_before_start() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let source_path = temp_workflow_path("accept-fixture-invalid-shape-workflow");
    fs::write(
        &source_path,
        r#"
workflow AcceptFixtureInvalidShape

rule noop
  when started
=> {}
"#,
    )
    .expect("write source");

    let cases = [
        (
            "missing-expect",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path
            }),
            "requires object field `expect`",
        ),
        (
            "non-object-expect",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "expect": []
            }),
            "expect must be an object",
        ),
        (
            "non-array-actions",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "actions": {"type": "pause"},
                "expect": {}
            }),
            "actions must be an array",
        ),
        (
            "non-array-provider-config-paths",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "provider_config_paths": "providers.json",
                "expect": {}
            }),
            "`provider_config_paths` must be an array of strings",
        ),
        (
            "non-string-provider",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "provider": ["fixture"],
                "expect": {}
            }),
            "`provider` must be a string",
        ),
        (
            "non-string-root",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "root": ["AcceptFixtureInvalidShape"],
                "expect": {}
            }),
            "`root` must be a string",
        ),
        (
            "non-string-outcome",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "outcome": ["completed"],
                "expect": {}
            }),
            "`outcome` must be a string",
        ),
        (
            "unknown-expect-dev-status",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "expect": {
                    "dev_status": "maybe"
                }
            }),
            "unknown expect.dev_status `maybe`",
        ),
        (
            "non-integer-expect-diagnostics",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "expect": {
                    "diagnostics": "0"
                }
            }),
            "`expect.diagnostics` must be a non-negative integer",
        ),
        (
            "non-array-expect-facts",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "expect": {
                    "facts": {}
                }
            }),
            "`expect.facts` must be an array",
        ),
        (
            "non-object-expect-summary",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "expect": {
                    "summary": []
                }
            }),
            "`expect.summary` must be an object",
        ),
        (
            "non-integer-expect-assertions-total",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "expect": {
                    "assertions": {
                        "total": "1"
                    }
                }
            }),
            "`expect.assertions.total` must be a non-negative integer",
        ),
        (
            "non-array-source-metadata-targets",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "expect": {
                    "source_metadata": {
                        "targets": {}
                    }
                }
            }),
            "`expect.source_metadata.targets` must be an array",
        ),
        (
            "non-object-trace-summary",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "expect": {
                    "trace": {
                        "summary": []
                    }
                }
            }),
            "`expect.trace.summary` must be an object",
        ),
        (
            "non-bool-trace-conformance-ok",
            json!({
                "schema": "whipplescript.acceptance_fixture.v0",
                "workflow": source_path,
                "expect": {
                    "trace": {
                        "conformance": {
                            "ok": "true"
                        }
                    }
                }
            }),
            "`expect.trace.conformance.ok` must be a boolean",
        ),
    ];

    for (label, fixture, expected_error) in cases {
        let store_path = temp_store_path();
        let fixture_path = temp_workflow_path(&format!("accept-fixture-invalid-shape-{label}"))
            .with_extension("json");
        fs::write(&fixture_path, fixture.to_string()).expect("write fixture");

        let output = Command::new(bin)
            .args([
                "--store",
                store_path.to_str().expect("utf-8 temp path"),
                "accept",
                fixture_path.to_str().expect("utf-8 fixture path"),
            ])
            .output()
            .expect("command runs");

        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected_error), "{stderr}");

        let _ = fs::remove_file(store_path);
        let _ = fs::remove_file(fixture_path);
    }

    let _ = fs::remove_file(source_path);
}

#[test]
fn accept_reports_observation_expectation_mismatches() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("accept-fixture-mismatch-workflow");
    let fixture_path = temp_workflow_path("accept-fixture-mismatch").with_extension("json");
    fs::write(
        &source_path,
        r#"
workflow AcceptFixtureMismatch

class Seen {
  status "ok"
}

assert count(Seen) == 1

rule seed
  when started
=> {
  record Seen {
    status "ok"
  }
}
"#,
    )
    .expect("write source");
    fs::write(
        &fixture_path,
        json!({
            "schema": "whipplescript.acceptance_fixture.v0",
            "workflow": source_path,
            "provider": "fixture",
            "expect": {
                "dev_status": "success",
                "workflow": "AcceptFixtureMismatch",
                "summary": {
                    "facts": 2,
                    "effects": 1
                },
                "actions": [
                    {"type": "pause", "count": 1}
                ],
                "trace": {
                    "conformance": {"ok": true},
                    "summary": {
                        "events": 999,
                        "abstract_events": 999
                    },
                    "groups": [
                        {"type": "instance_paused", "count": 1}
                    ],
                    "items": [
                        {},
                        {"sequence": 1, "type": "effect_terminal", "status": "completed"}
                    ]
                },
                "assertion_reads": [
                    {},
                    {
                        "source": "effect:kind agent.tell where status == completed",
                        "match_count": 1
                    }
                ],
                "assertion_tags": [
                    {"tag": "acceptance", "total": 1}
                ],
                "source_metadata": {
                    "targets": [
                        {
                            "target_kind": "workflow",
                            "target": "AcceptFixtureMismatch",
                            "tags": ["acceptance"]
                        }
                    ]
                },
                "assertion_untagged": {
                    "total": 2,
                    "passed": 2
                },
                "facts": [
                    {"name": "Seen", "count": 2}
                ],
                "runs": [
                    {"provider": "fixture", "status": "completed", "count": 1}
                ],
                "artifacts": [
                    {"kind": "transcript", "mime_type": "text/plain", "count": 1}
                ],
                "evidence": [
                    {"kind": "agent.turn.native_event", "subject_type": "run", "count": 1}
                ],
                "inbox": [
                    {"status": "pending", "severity": "normal", "count": 1}
                ]
            }
        })
        .to_string(),
    )
    .expect("write fixture");

    let output = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "accept",
            fixture_path.to_str().expect("utf-8 fixture path"),
        ])
        .output()
        .expect("command runs");
    assert!(
        !output.status.success(),
        "acceptance mismatch should fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON report");
    assert_eq!(report.get("passed").and_then(Value::as_bool), Some(false));
    let failures = report
        .get("failures")
        .and_then(Value::as_array)
        .expect("failures");
    assert!(failures.iter().any(|failure| failure
        .as_str()
        .is_some_and(|failure| failure.contains("expected facts[0] name=\"Seen\" count=2"))));
    assert!(failures.iter().any(|failure| failure
        .as_str()
        .is_some_and(|failure| failure.contains("expected summary.facts=2, got 1"))));
    assert!(failures.iter().any(|failure| failure
        .as_str()
        .is_some_and(|failure| failure.contains("expected summary.effects=1, got 0"))));
    assert!(failures
        .iter()
        .any(|failure| failure.as_str().is_some_and(|failure| failure
            .contains("expected assertion_tags[0] tag=\"acceptance\", got no matching"))));
    assert!(failures.iter().any(|failure| failure.as_str().is_some_and(
        |failure| failure.contains("expected source_metadata.targets[0] \"workflow:AcceptFixtureMismatch\", got no matching target")
    )));
    assert!(failures.iter().any(|failure| failure
        .as_str()
        .is_some_and(|failure| failure.contains("expected assertion_untagged.total=2, got 1"))));
    assert!(failures.iter().any(|failure| failure
        .as_str()
        .is_some_and(|failure| failure.contains("expected assertion_untagged.passed=2, got 1"))));
    assert!(failures
        .iter()
        .any(
            |failure| failure.as_str().is_some_and(|failure| failure.contains(
                "expected runs[0] provider=\"fixture\" status=\"completed\" count=1, got 0"
            ))
        ));
    assert!(failures.iter().any(|failure| failure
        .as_str()
        .is_some_and(|failure| failure.contains(
            "expected artifacts[0] kind=\"transcript\" mime_type=\"text/plain\" count=1, got 0"
        ))));
    assert!(failures.iter().any(|failure| failure.as_str().is_some_and(
        |failure| failure.contains("expected evidence[0] kind=\"agent.turn.native_event\" subject_type=\"run\" count=1, got 0")
    )));
    assert!(failures
        .iter()
        .any(|failure| failure.as_str().is_some_and(|failure| failure
            .contains("expected inbox[0] status=\"pending\" severity=\"normal\" count=1, got 0"))));
    assert!(failures.iter().any(|failure| failure.as_str().is_some_and(
        |failure| failure.contains("expected actions[0] type=\"pause\" count=1, got 0")
    )));
    assert!(failures
        .iter()
        .any(|failure| failure.as_str().is_some_and(|failure| failure
            .contains("expected trace.groups[0] type=\"instance_paused\" count=1, got 0"))));
    assert!(failures.iter().any(|failure| failure.as_str().is_some_and(
        |failure| failure.contains("expect.trace.items[0] must include at least one selector")
    )));
    assert!(failures.iter().any(|failure| failure.as_str().is_some_and(
        |failure| failure.contains(
            "expected trace.items[1] sequence=1 type=\"effect_terminal\" status=\"completed\", got no matching trace item"
        )
    )));
    assert!(failures.iter().any(|failure| failure
        .as_str()
        .is_some_and(|failure| failure.contains("expected trace.summary.events=999"))));
    assert!(failures.iter().any(|failure| failure
        .as_str()
        .is_some_and(|failure| failure.contains("expected trace.summary.abstract_events=999"))));
    assert!(failures
        .iter()
        .any(|failure| failure.as_str().is_some_and(|failure| failure
            .contains("expect.assertion_reads[0] must include at least one selector"))));
    assert!(failures.iter().any(|failure| failure.as_str().is_some_and(
        |failure| failure.contains(
            "expected assertion_reads[1] source=\"effect:kind agent.tell where status == completed\", got no matching assertion read"
        )
    )));
    assert_eq!(
        report
            .pointer("/observed/facts/0/count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/summary/facts")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/summary/effects")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        report
            .pointer("/observed/executable_spec/summary/total")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/executable_spec/untagged/summary/total")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/actions")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(fixture_path);
}

#[test]
fn accept_can_expect_failed_executable_spec_diagnostics() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("accept-fixture-expected-failure-workflow");
    let fixture_path = temp_workflow_path("accept-fixture-expected-failure").with_extension("json");
    fs::write(
        &source_path,
        r#"
workflow AcceptFixtureExpectedFailure

class Seen {
  status "ok"
}

@acceptance
assert count(Seen) == 2

rule seed
  when started
=> {
  record Seen {
    status "ok"
  }
}
"#,
    )
    .expect("write source");
    fs::write(
        &fixture_path,
        json!({
            "schema": "whipplescript.acceptance_fixture.v0",
            "workflow": source_path,
            "provider": "fixture",
            "include_tags": ["acceptance"],
            "expect": {
                "dev_status": "failure",
                "workflow": "AcceptFixtureExpectedFailure",
                "status": "failed",
                "diagnostics": 1,
                "diagnostics_by_code": [
                    {"code": "assertion.failed", "count": 1}
                ],
                "assertions": {
                    "total": 1,
                    "passed": 0,
                    "failed": 1,
                    "error": 0
                },
                "assertion_tags": [
                    {"tag": "acceptance", "total": 1, "passed": 0, "failed": 1, "error": 0}
                ],
                "summary": {
                    "facts": 1,
                    "effects": 0
                },
                "facts": [
                    {"name": "Seen", "count": 1}
                ]
            }
        })
        .to_string(),
    )
    .expect("write fixture");

    let report = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "accept",
            fixture_path.to_str().expect("utf-8 fixture path"),
        ],
    );

    assert_eq!(
        report.get("schema").and_then(Value::as_str),
        Some("whipplescript.acceptance_report.v0")
    );
    assert_eq!(report.get("passed").and_then(Value::as_bool), Some(true));
    assert_eq!(
        report
            .get("failures")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        report
            .pointer("/dev_report/executable_spec/status")
            .and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        report
            .pointer("/dev_report/diagnostics/0/code")
            .and_then(Value::as_str),
        Some("assertion.failed")
    );
    assert_eq!(
        report
            .pointer("/observed/executable_spec/status")
            .and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        report
            .pointer("/observed/executable_spec/tags/0/summary/failed")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/observed/diagnostics_by_code/0/code")
            .and_then(Value::as_str),
        Some("assertion.failed")
    );
    assert_eq!(
        report
            .pointer("/observed/diagnostics_by_code/0/count")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        report
            .pointer("/dev_report/executable_spec/tags/0/summary/failed")
            .and_then(Value::as_u64),
        Some(1)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(fixture_path);
}

#[test]
fn dev_reports_static_assertion_errors_with_nonzero_exit() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("assertion-error");
    fs::write(
        &source_path,
        r#"
workflow AssertionError

assert missing.value
"#,
    )
    .expect("write source");

    let output = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ])
        .output()
        .expect("command runs");
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "static assertion diagnostics should not emit dev JSON\nstdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("assertion has unknown expression root `missing`"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("use a binding introduced by a `when ... as name` clause"),
        "stderr:\n{stderr}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

fn run_json(bin: &str, args: &[&str]) -> Value {
    let text = run_text(bin, args);
    serde_json::from_str(&text).expect("valid JSON output")
}

fn run_text(bin: &str, args: &[&str]) -> String {
    let output = Command::new(bin).args(args).output().expect("command runs");
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout is utf-8")
}

fn ticket(status: &Value) -> Option<&str> {
    status
        .get("instance")?
        .get("input")?
        .get("ticket")?
        .as_str()
}

fn example_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

fn temp_store_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "whipplescript-control-plane-{}-{nanos}.sqlite",
        std::process::id()
    ))
}

fn temp_workflow_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "whipplescript-{label}-{}-{nanos}.whip",
        std::process::id()
    ))
}
