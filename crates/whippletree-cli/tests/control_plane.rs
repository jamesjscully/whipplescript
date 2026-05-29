use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use whippletree_store::{NewFact, RuleCommit, SqliteStore};

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
        "human-review.whip",
        "implementation-plan-phase-review.whip",
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
        Some("whippletree.local_trace.v0")
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

use plugin "memory"

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
    assert_eq!(assertions.len(), 5);
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

#[test]
fn dev_branches_on_completed_terminal_union_payload() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("terminal-completed-branch");
    fs::write(
        &source_path,
        r#"
workflow TerminalCompletedBranch

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

coerce classifyMessage(title string, body string) -> MessageClassification {
  prompt "Classify"
}

assert count(TerminalRoute where branch == "completed") == 1
assert count(TerminalRoute where detail == "Fixture classification") == 1

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
      }
      Failed failure => {
        record TerminalRoute {
          branch "failed"
          detail failure.reason
        }
      }
      TimedOut timeout => {
        record TerminalRoute {
          branch "timed_out"
          detail timeout.summary
        }
      }
      Cancelled cancel => {
        record TerminalRoute {
          branch "cancelled"
          detail cancel.summary
        }
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
            effects: &[],
            dependencies: &[],
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
        "whippletree-control-plane-{}-{nanos}.sqlite",
        std::process::id()
    ))
}

fn temp_workflow_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "whippletree-{label}-{}-{nanos}.whip",
        std::process::id()
    ))
}
