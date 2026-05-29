use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

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
