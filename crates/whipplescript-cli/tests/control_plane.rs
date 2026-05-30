use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use whipplescript_store::{EffectCompletion, NewFact, RuleCommit, RunStart, SqliteStore};

#[test]
fn checks_all_example_workflows() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let examples = [
        "minimal-noop.whip",
        "ralph.whip",
        "loft-worker-with-review.whip",
        "coerce-branch.whip",
        "codex-french-poem-dogfood.whip",
        "codex-poem-coerce-review.whip",
        "companion-skill-dogfood.whip",
        "human-review.whip",
        "implementation-plan-phase-review.whip",
        "multi-provider-poem-review.whip",
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
fn check_root_option_validates_current_workflow_name() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let source_path = temp_workflow_path("root-selection");
    fs::write(
        &source_path,
        r#"
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

workflow Alpha {
  rule alpha
    when started
  => {
    record Selected {
      id "alpha"
    }
  }
}

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
fn dev_phase_review_creates_requests_and_runs_fixture_turns() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("implementation-plan-phase-review.whip");
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
        Some(14)
    );
    assert_eq!(
        first_step.get("effects_created").and_then(Value::as_u64),
        Some(14)
    );
    let first_worker = dev
        .get("workers")
        .and_then(Value::as_array)
        .and_then(|workers| workers.first())
        .expect("first worker");
    assert_eq!(
        first_worker.get("ran_effects").and_then(Value::as_u64),
        Some(14)
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
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("PhaseReviewRequest"))
            .count(),
        0
    );
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("PhaseReviewDispatch"))
            .count(),
        14
    );
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("agent.turn.completed"))
            .count(),
        14
    );

    let _ = fs::remove_file(store_path);
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
                .is_some_and(|message| message.contains("fixture failure"))
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
fn dev_loft_claim_success_releases_agent_turn_dependency() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("loft-claim");
    fs::write(
        &workflow_path,
        r#"
workflow LoftClaim

class WorkItem {
  title string
  body string
}

agent worker {
  profile "repo-writer"
  capacity 1
}

rule seed
  when started
=> {
  record WorkItem {
    title "Fix it"
    body "Please"
  }
}

rule start_issue
  when WorkItem as issue
  when worker is available
=> {
  claim issue with loft as claim

  after claim succeeds {
    tell worker """
    Implement {{ issue.title }}
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
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("loft.claim.succeeded")));
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("agent.turn.completed")));

    let _ = fs::remove_file(store_path);
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
fn dev_event_emit_fixture_materializes_heartbeat_fact() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("event-emit");
    fs::write(
        &workflow_path,
        r#"
workflow EventEmit

class Tick {
  status "ready"
}

class HeartbeatSeen {
  event string
  status "observed"
}

assert none(Tick where status == "ready")
assert one(HeartbeatSeen where status == "observed")
assert one(effect kind event.emit where status == completed)

rule seed
  when started
=> {
  record Tick {
    status "ready"
  }
}

rule emit_heartbeat
  when Tick as tick where tick.status == "ready"
=> {
  emit openclaw.heartbeat as heartbeat

  after heartbeat succeeds as emitted => {
    done tick -> record HeartbeatSeen {
      event emitted.event_type
      status "observed"
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
                == Some("openclaw.heartbeat"))
    );

    let _ = fs::remove_file(store_path);
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

  rule accept_input
    when PhaseRequest as phase
  => {
    emit phaseAccepted {
      phaseId phase.phaseId
      title phase.title
    } as accepted
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
        fact.get("name").and_then(Value::as_str) == Some("phaseAccepted")
            && fact
                .get("value")
                .and_then(|value| value.get("value"))
                .and_then(|value| value.get("bindings"))
                .and_then(|value| value.get("phase"))
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
pattern EmitSeen<Input> {
  rule dispatch
    when Input as item
  => {
    emit eventName as seen
  }
}

workflow PatternApplication {
  input task Task

  class Task {
    title string
  }

  apply EmitSeen<Task> as taskSeen {
    eventName seen
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
        fact.get("name").and_then(Value::as_str) == Some("seen")
            && fact
                .get("value")
                .and_then(|value| value.get("value"))
                .and_then(|value| value.get("bindings"))
                .and_then(|value| value.get("item"))
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
        fact.get("name").and_then(Value::as_str) == Some("workflow.invoke.failed")
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
fn dev_codex_then_coerce_rehydrates_after_bound_baml_arguments() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("codex-poem-coerce-review.whip");
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
        vec![1, 1, 0, 0]
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
        arguments.get("arg0").and_then(Value::as_str),
        Some("rain over a city at night")
    );
    assert_eq!(
        arguments.get("arg1").and_then(Value::as_str),
        Some("target/dogfood/coerce-french-poem.txt")
    );
    assert_eq!(
        arguments.get("arg2").and_then(Value::as_str),
        Some("fixture completed")
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
        fact.get("name").and_then(Value::as_str) == Some("ReviewedPoem")
            && fact
                .get("value")
                .and_then(|value| value.get("review"))
                .and_then(|review| review.get("isFrench"))
                .and_then(Value::as_bool)
                == Some(true)
    }));

    let _ = fs::remove_file(store_path);
}

#[test]
fn dev_provider_language_e2e_runs_agent_matrix_and_baml_reviews() {
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
    let assertions = dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions");
    assert_eq!(assertions.len(), 6);
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
fn dev_companion_skill_dogfood_routes_with_agentref_metadata() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("companion-skill-dogfood.whip");
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
    assert_eq!(assertions.len(), 6);
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
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("CompanionReviewDispatch"))
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
        ["codex", "claude", "pi"]
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
    run_terminal_branch_workflow(bin, Some("--fail"), "failed", "fixture coerce failure");
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
assert empty(ExprResult where provider == pi)
assert count(ExprResult where priority > 1 && provider in ["codex", "claude"]) == 1
assert ("codex" in ["codex", "claude"]) && !("pi" in ["codex"])
assert empty([])

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
workflow DurationTimeOrderingCheck

class Window {
  elapsed duration
  limit duration
  opened_at time
  due_at time
}

rule duration_guard
  when Window as window where window.elapsed < window.limit
=> {
}

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

assert empty(Result)

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
assert empty(MaybeOwner where metadata["missing"] == null)
assert empty(MaybeOwner where exists metadata["missing"])

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
        arguments
            .pointer("/arg0/owner/name")
            .and_then(Value::as_str),
        Some("Ada")
    );
    assert_eq!(
        arguments.pointer("/arg1/phase").and_then(Value::as_str),
        Some("kernel")
    );

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
        diagnostic.get("code").and_then(Value::as_str) == Some("assertion.failed")
            && diagnostic.get("subject_type").and_then(Value::as_str) == Some("assertion")
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

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
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
