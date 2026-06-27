//! B1 "soft middle" regression suite: anything `check` accepts must execute
//! faithfully. Each test here pins a failure mode where bodies previously
//! compiled and then silently did the wrong thing at runtime.

use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

fn run_json(bin: &str, args: &[&str]) -> Value {
    let output = Command::new(bin).args(args).output().expect("command runs");
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    let json_start = stdout.find(['{', '[']).expect("json output");
    serde_json::from_str(&stdout[json_start..]).expect("valid json")
}

fn temp_path(label: &str, extension: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "whipplescript-soft-middle-{label}-{}-{nanos}.{extension}",
        std::process::id()
    ))
}

fn dev_until_idle(bin: &str, store: &str, source: &str, extra: &[&str]) -> Value {
    let mut args = vec!["--store", store, "--json", "dev", source];
    args.extend_from_slice(extra);
    args.extend_from_slice(&["--provider", "fixture", "--until", "idle"]);
    run_json(bin, &args)
}

fn instance_status(bin: &str, store: &str, instance: &str) -> String {
    let status = run_json(bin, &["--store", store, "--json", "status", instance]);
    status
        .pointer("/instance/status")
        .and_then(Value::as_str)
        .expect("instance status")
        .to_owned()
}

/// Filtered fact queries behave identically in guards and assertions, and
/// single-line table rows seed exactly like multi-line rows.
#[test]
fn filtered_query_guards_match_assertions_over_single_line_rows() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("guard-assert", "sqlite");
    let source = temp_path("guard-assert", "whip");
    fs::write(
        &source,
        r#"
workflow GuardAssertEquivalence

output result Report

class Item {
  id string
  status string
}

class Report {
  total int
}

table items as Item [
  { id "a" status "done" }
  { id "b" status "done" }
  {
    id "c"
    status "open"
  }
]

rule finish
  when Item as item where count(Item where status == "done") == 2 and exists(Item where status == "open")
=> {
  complete result {
    total count(Item where status == "done")
  }
}

assert count(Item where status == "done") == 2
assert exists(Item where status == "open")
"#,
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    for assertion in dev
        .get("assertions")
        .and_then(Value::as_array)
        .expect("assertions")
    {
        assert_eq!(
            assertion.get("status").and_then(Value::as_str),
            Some("passed"),
            "{assertion}"
        );
    }
    assert_eq!(instance_status(bin, store_str, instance), "completed");

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// Templates in record/payload string fields render against bindings.
#[test]
fn templates_in_payload_fields_render() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("templates", "sqlite");
    let source = temp_path("templates", "whip");
    fs::write(
        &source,
        r#"
workflow Templates

output result Out

class In {
  a string
  b string
}

class Out {
  joined string
}

table seeds as In [
  { a "x" b "y" }
]

rule go
  when In as i
=> {
  complete result {
    joined "{{ i.a }}-{{ i.b }}"
  }
}
"#,
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let log = run_json(bin, &["--store", store_str, "--json", "log", instance]);
    let completed = log
        .as_array()
        .expect("log")
        .iter()
        .find(|event| event.get("event_type").and_then(Value::as_str) == Some("workflow.completed"))
        .expect("completed event");
    assert_eq!(
        completed
            .pointer("/payload/payload/joined")
            .and_then(Value::as_str),
        Some("x-y")
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// Arithmetic in field values evaluates (retry-counter idiom).
#[test]
fn arithmetic_in_field_values_evaluates_on_failure_path() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("arith", "sqlite");
    let source = temp_path("arith", "whip");
    fs::write(
        &source,
        r#"
workflow Arithmetic

failure error GaveUp

class Job {
  id string
  attempts int
}

class GaveUp {
  attempts int
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

table jobs as Job [
  { id "j" attempts 0 }
]

rule attempt
  when Job as job where job.attempts < 1
  when worker is available
=> {
  tell worker as turn "do it"

  after turn fails as failed {
    done job -> record Job {
      id job.id
      attempts job.attempts + 1
    }
  }
}

rule give_up
  when Job as job where job.attempts >= 1
=> {
  fail error {
    attempts job.attempts * 10
  }
}
"#,
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &["--fail"]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "failed");
    let log = run_json(bin, &["--store", store_str, "--json", "log", instance]);
    let failed = log
        .as_array()
        .expect("log")
        .iter()
        .find(|event| event.get("event_type").and_then(Value::as_str) == Some("workflow.failed"))
        .expect("failed event");
    assert_eq!(
        failed
            .pointer("/payload/payload/attempts")
            .and_then(Value::as_i64),
        Some(10)
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// 503 auto-fail: an effect whose failure is unhandled in a self-terminating flow
/// drives the workflow to a `failed` terminal (instead of stalling forever) via the
/// generic internal-failure path — no author `on fails` handler, no typed `failure`
/// payload. Modeled in models/maude/flow-autofail.maude.
#[test]
fn unhandled_flow_failure_auto_fails_the_workflow() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("autofail", "sqlite");
    let source = temp_path("autofail", "whip");
    fs::write(
        &source,
        r#"
workflow AutoFail

output result Decision
failure error Blocked

class Trigger {
  id string
}

class Decision {
  ok string
}

class Blocked {
  reason string
}

agent worker {
  provider fixture
  profile "repo-reader"
  capacity 1
}

table seed as Trigger [
  { id "t" }
]

flow f
  when Trigger as t
{
  tell worker as turn "do it"

  complete result {
    ok "done"
  }
}
"#,
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &["--fail"]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    // The `tell` has no `on fails` handler. Without auto-fail its failure would
    // leave the instance stuck `running`; auto-fail drives it to `failed`.
    assert_eq!(instance_status(bin, store_str, instance), "failed");
    let log = run_json(bin, &["--store", store_str, "--json", "log", instance]);
    let transitioned = log
        .as_array()
        .expect("log")
        .iter()
        .find(|event| {
            event.get("event_type").and_then(Value::as_str) == Some("instance.transitioned")
                && event.pointer("/payload/status").and_then(Value::as_str) == Some("failed")
        })
        .expect("instance.transitioned failed event");
    let reason = transitioned
        .pointer("/payload/reason")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        reason.contains("unhandled failure"),
        "generic auto-fail reason expected, got: {reason}"
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// A self-terminating flow whose effect failure IS handled by an author `on fails`
/// handler must fail through the typed terminal (a `workflow.failed` with the
/// declared payload), NOT the generic auto-fail path.
#[test]
fn handled_flow_failure_does_not_auto_fail() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("handledfail", "sqlite");
    let source = temp_path("handledfail", "whip");
    fs::write(
        &source,
        r#"
workflow HandledFail

output result Decision
failure error Blocked

class Trigger {
  id string
}

class Decision {
  ok string
}

class Blocked {
  reason string
}

agent worker {
  provider fixture
  profile "repo-reader"
  capacity 1
}

table seed as Trigger [
  { id "t" }
]

flow f
  when Trigger as t
{
  tell worker as turn "do it"
  on fails {
    fail error {
      reason "handled"
    }
  }

  complete result {
    ok "done"
  }
}
"#,
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &["--fail"]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "failed");
    let log = run_json(bin, &["--store", store_str, "--json", "log", instance]);
    // The typed handler fires a workflow.failed with the declared Blocked payload;
    // the generic auto-fail transition must NOT appear.
    let failed = log
        .as_array()
        .expect("log")
        .iter()
        .find(|event| event.get("event_type").and_then(Value::as_str) == Some("workflow.failed"))
        .expect("workflow.failed event");
    assert_eq!(
        failed
            .pointer("/payload/payload/reason")
            .and_then(Value::as_str),
        Some("handled"),
        "the author on-fails handler payload must fire: {failed}"
    );
    assert!(
        !log.as_array().expect("log").iter().any(|event| {
            event.get("event_type").and_then(Value::as_str) == Some("instance.transitioned")
                && event
                    .pointer("/payload/reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .contains("unhandled failure")
        }),
        "a handled failure must not also auto-fail: {log}"
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// 1929 OPTION A: `send via <channel>` (std.messaging) compiles with NO package
/// lock (the std-library exemption), runs as a `messaging.send` capability.call
/// under the fixture provider, and the workflow completes on `after sent succeeds`.
#[test]
fn send_via_channel_runs_under_fixture_and_completes() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("send", "sqlite");
    let source = temp_path("send", "whip");
    fs::write(
        &source,
        r##"
@service
workflow Notify

output result Done

class Trigger {
  id string
}

class Done {
  ok string
}

agent worker {
  provider fixture
  profile "repo-reader"
  capacity 1
}

channel alerts {
  provider slack
  destination "#ops"
}

table seed as Trigger [
  { id "t" }
]

rule notify
  when Trigger as t
=> {
  send via alerts {
    text "hello"
  } as sent

  after sent succeeds {
    complete result {
      ok "sent"
    }
  }
}
"##,
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    // No `--package-lock`: the std-library exemption must let `send` compile + run.
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "completed");
    let effects = run_json(bin, &["--store", store_str, "--json", "effects", instance]);
    let send = effects
        .as_array()
        .expect("effects")
        .iter()
        .find(|effect| effect.get("target").and_then(Value::as_str) == Some("messaging.send"))
        .expect("messaging.send effect");
    assert_eq!(
        send.get("status").and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        send.pointer("/input/message/channel")
            .and_then(Value::as_str),
        Some("alerts")
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// Inbound messaging (spec/messaging.md): `whip message` injects a `Message` on a
/// declared channel and a `when message from <channel> as msg` rule fires, binding
/// the envelope. The fixture-parity counterpart of outbound `send`; live providers
/// stay gated.
#[test]
fn inbound_message_fires_when_message_from_rule() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("inbound", "sqlite");
    let source = temp_path("inbound", "whip");
    fs::write(
        &source,
        r#"
@service
workflow Inbound

use std.messaging

channel release_room {
  provider slack
}

output result Decision

class Decision {
  note string
}

rule react
  when message from release_room as msg
=> {
  complete result {
    note msg.text
  }
}
"#,
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let source_str = source.to_str().expect("utf-8");
    // The instance starts running and waits — no message has arrived yet.
    let dev = dev_until_idle(bin, store_str, source_str, &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();
    assert_eq!(instance_status(bin, store_str, &instance), "running");

    // Inject an inbound message on the channel, then step the reactive rule.
    run_text(
        bin,
        &[
            "--store",
            store_str,
            "message",
            &instance,
            "--channel",
            "release_room",
            "--text",
            "ship it",
            "--program",
            source_str,
        ],
    );
    run_text(
        bin,
        &[
            "--store",
            store_str,
            "step",
            &instance,
            "--program",
            source_str,
        ],
    );
    // Completing proves the `msg` binding resolved (an unresolved `msg.text`
    // would fail the rule rather than complete).
    assert_eq!(instance_status(bin, store_str, &instance), "completed");

    // The injected envelope is a well-formed `Message` fact on the channel.
    let facts = run_json(bin, &["--store", store_str, "--json", "facts", &instance]);
    let message = facts
        .as_array()
        .expect("facts")
        .iter()
        .find(|fact| fact.get("name").and_then(Value::as_str) == Some("message.release_room"))
        .expect("message fact");
    assert_eq!(
        message.pointer("/value/text").and_then(Value::as_str),
        Some("ship it")
    );
    assert_eq!(
        message.pointer("/value/channel").and_then(Value::as_str),
        Some("release_room")
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// The general readiness form `when fact <dotted.name> as x` matches runtime
/// facts; the English sugar phrases are abbreviations of it.
#[test]
fn general_fact_match_fires_rule() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("general-fact", "sqlite");
    let source = temp_path("general-fact", "whip");
    fs::write(
        &source,
        r#"
workflow GeneralFact

output result Seen

class Seen {
  agent string
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
  tell worker "go"
}

rule observe
  when fact agent.turn.completed as turn where turn.agent == "worker"
=> {
  complete result {
    agent turn.agent
  }
}
"#,
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "completed");

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

fn run_text(bin: &str, args: &[&str]) {
    let output = Command::new(bin).args(args).output().expect("command runs");
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The ask + timer escalation race: the timer fires, the rule cancels the
/// losing ask, and the workflow fails on the deadline branch.
#[test]
fn timer_fires_and_cancel_settles_the_race() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("timer-race", "sqlite");
    let source = temp_path("timer-race", "whip");
    fs::write(
        &source,
        r#"
workflow TimerRace

output result Done
failure error TimedOut

class Done {
  decision string
}

class TimedOut {
  reason string
}

rule begin
  when started
=> {
  askHuman as signoff "Approve the plan?"
  timer 3s as deadline

  after deadline succeeds {
    cancel signoff
    fail error {
      reason "no answer within deadline"
    }
  }
}

rule approve
  when human answered signoff as answer where answer.choice == "approve"
=> {
  complete result {
    decision answer.choice
  }
}
"#,
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let source_str = source.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source_str, &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();
    // The `timer 3s` gives `dev_until_idle` ample margin to settle to a waiting
    // instance before the deadline fires (a `timer 1s` raced the dev loop on
    // slower machines). Then sleep past the deadline so the timer is due.
    assert_eq!(instance_status(bin, store_str, &instance), "running");

    std::thread::sleep(std::time::Duration::from_secs(4));
    run_text(
        bin,
        &[
            "--store",
            store_str,
            "worker",
            &instance,
            "--provider",
            "fixture",
        ],
    );
    run_text(
        bin,
        &[
            "--store",
            store_str,
            "step",
            &instance,
            "--program",
            source_str,
        ],
    );
    assert_eq!(instance_status(bin, store_str, &instance), "failed");

    let effects = run_json(bin, &["--store", store_str, "--json", "effects", &instance]);
    let timer = effects
        .as_array()
        .expect("effects")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("timer.wait"))
        .expect("timer effect");
    assert_eq!(
        timer.get("status").and_then(Value::as_str),
        Some("completed")
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// A `timeout` clause expires a never-run effect to `timed_out` and the
/// failure branch fires.
#[test]
fn timeout_clause_expires_queued_effect() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("deadline", "sqlite");
    let source = temp_path("deadline", "whip");
    fs::write(
        &source,
        r#"
workflow Deadline

failure error TooSlow

class TooSlow {
  reason string
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
  tell worker as turn timeout 1s "do something slow"

  after turn fails as failed {
    fail error {
      reason "agent missed the deadline"
    }
  }
}
"#,
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let source_str = source.to_str().expect("utf-8");
    let started = run_json(bin, &["--store", store_str, "--json", "run", source_str]);
    let instance = started
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();
    run_text(
        bin,
        &[
            "--store",
            store_str,
            "step",
            &instance,
            "--program",
            source_str,
        ],
    );

    std::thread::sleep(std::time::Duration::from_secs(2));
    run_text(
        bin,
        &[
            "--store",
            store_str,
            "worker",
            &instance,
            "--provider",
            "fixture",
        ],
    );
    run_text(
        bin,
        &[
            "--store",
            store_str,
            "step",
            &instance,
            "--program",
            source_str,
        ],
    );
    assert_eq!(instance_status(bin, store_str, &instance), "failed");

    let effects = run_json(bin, &["--store", store_str, "--json", "effects", &instance]);
    let tell = effects
        .as_array()
        .expect("effects")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("agent.tell"))
        .expect("tell effect");
    assert_eq!(
        tell.get("status").and_then(Value::as_str),
        Some("timed_out")
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

const TRIAGE_FLOW: &str = r#"
workflow TicketTriage

input ticket Ticket
output result TriageDecision
failure error TriageBlocked

class Ticket {
  id string
  title string
}

class TriageDecision {
  decision string
  decidedBy string
}

class TriageBlocked {
  reason string
}

agent triager {
  provider fixture
  profile "repo-reader"
  capacity 1
}

flow triage
  when Ticket as ticket
{
  tell triager as turn """markdown
  Suggest a fix plan for {{ ticket.title }}.
  """

  askHuman as signoff """markdown
  Plan for {{ ticket.title }}: {{ turn.summary }} — approve or reject.
  """

  when signoff.choice == "approve" {
    complete result {
      decision signoff.choice
      decidedBy signoff.answered_by
    }
  } else {
    fail error {
      reason "rejected"
    }
  }
}
"#;

fn drive_triage_flow(choice: &str) -> String {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path(&format!("flow-{choice}"), "sqlite");
    let source = temp_path(&format!("flow-{choice}"), "whip");
    fs::write(&source, TRIAGE_FLOW).expect("write source");
    let store_str = store.to_str().expect("utf-8").to_owned();
    let source_str = source.to_str().expect("utf-8").to_owned();

    let dev = run_json(
        bin,
        &[
            "--store",
            &store_str,
            "--input",
            r#"{"ticket":{"id":"T-1","title":"Fix login"}}"#,
            "--json",
            "dev",
            &source_str,
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();

    let inbox = run_json(bin, &["--store", &store_str, "--json", "inbox"]);
    let item = inbox
        .as_array()
        .expect("inbox")
        .first()
        .and_then(|item| item.get("inbox_item_id"))
        .and_then(Value::as_str)
        .expect("pending ask")
        .to_owned();
    run_text(
        bin,
        &[
            "--store", &store_str, "inbox", "answer", &item, "--choice", choice, "--by", "alice",
        ],
    );
    run_text(
        bin,
        &[
            "--store",
            &store_str,
            "step",
            &instance,
            "--program",
            &source_str,
        ],
    );
    let status = instance_status(bin, &store_str, &instance);

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
    status
}

/// A flow chains tell -> askHuman -> branch with compiler-generated
/// correlation; approval completes the workflow.
#[test]
fn flow_approve_path_completes() {
    assert_eq!(drive_triage_flow("approve"), "completed");
}

/// The flow's else branch fails the workflow on rejection.
#[test]
fn flow_reject_path_fails() {
    assert_eq!(drive_triage_flow("reject"), "failed");
}

/// A hand-written `after <ask> completes { case <ask> { Completed decided => ...
/// decided.choice } }` must resolve the human answer's fields at runtime. The
/// scheduled-escalation example answers before its deadline, so the Completed
/// branch fires and the workflow completes carrying the answered choice — a
/// regression guard for the human-ask terminal binding (the answer fact, not the
/// `human.ask.issued` ack, must back the case scrutinee).
#[test]
fn human_ask_case_completed_branch_resolves_answer_choice() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("human-case", "sqlite");
    let store_str = store.to_str().expect("utf-8").to_owned();
    // Reuse the scheduled-escalation example (its `case answer { Completed
    // decided => ... decided.choice }` is exactly the path under test), but push
    // the deadline far into the future so the human answer wins the race and the
    // Completed branch fires deterministically.
    let example = format!(
        "{}/../../examples/scheduled-escalation.whip",
        env!("CARGO_MANIFEST_DIR")
    );
    let source_path = temp_path("human-case", "whip");
    let source = source_path.to_str().expect("utf-8").to_owned();
    let text = fs::read_to_string(&example)
        .expect("read example")
        .replace("2026-06-11T18:00:00Z", "2999-01-01T00:00:00Z");
    fs::write(&source_path, text).expect("write source");

    let dev = run_json(
        bin,
        &[
            "--store",
            &store_str,
            "--json",
            "dev",
            &source,
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();

    let inbox = run_json(bin, &["--store", &store_str, "--json", "inbox"]);
    let item = inbox
        .as_array()
        .expect("inbox")
        .first()
        .and_then(|item| item.get("inbox_item_id"))
        .and_then(Value::as_str)
        .expect("pending ask")
        .to_owned();
    run_text(
        bin,
        &[
            "--store", &store_str, "inbox", "answer", &item, "--choice", "approve", "--by", "alice",
        ],
    );
    run_text(
        bin,
        &[
            "--store",
            &store_str,
            "step",
            &instance,
            "--program",
            &source,
        ],
    );

    // Answering before the deadline takes the Completed branch and completes.
    assert_eq!(instance_status(bin, &store_str, &instance), "completed");
    // `decided.choice` resolved to the answered choice in the completion output.
    let log = run_json(bin, &["--store", &store_str, "--json", "log", &instance]);
    let decision = log
        .as_array()
        .expect("log")
        .iter()
        .find(|event| event.get("event_type").and_then(Value::as_str) == Some("workflow.completed"))
        .and_then(|event| event.get("payload"))
        .and_then(|payload| payload.get("payload"))
        .and_then(|payload| payload.get("decision"))
        .and_then(Value::as_str);
    assert_eq!(decision, Some("approve"));

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source_path);
}

/// Flows fan out per matched fact: two tickets, two progressions.
#[test]
fn flow_fans_out_per_matched_fact() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("flow-fan", "sqlite");
    let source = temp_path("flow-fan", "whip");
    fs::write(
        &source,
        r#"
@service
workflow FanTriage

class Ticket {
  id string
  title string
}

class Plan {
  id string
  plan string
}

agent triager {
  provider fixture
  profile "repo-reader"
  capacity 2
}

table tickets as Ticket [
  {
    id "T-1"
    title "Bug one"
  }
  {
    id "T-2"
    title "Bug two"
  }
]

flow plan_ticket
  when Ticket as ticket
{
  tell triager as turn """markdown
  Plan {{ ticket.title }}.
  """

  record Plan {
    id ticket.id
    plan turn.summary
  }
}
"#,
    )
    .expect("write source");
    let store_str = store.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let facts = run_json(bin, &["--store", store_str, "--json", "facts", instance]);
    let plans = facts
        .as_array()
        .expect("facts")
        .iter()
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("Plan"))
        .count();
    assert_eq!(plans, 2);

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

const EXEC_FLOW: &str = r#"
workflow ExecDemo

output result Ran
failure error Blocked

class Ran {
  out string
}

class Blocked {
  reason string
}

rule go
  when started
=> {
  exec "echo hello-from-exec" as run

  after run succeeds as out {
    complete result {
      out out.stdout
    }
  }

  after run fails {
    fail error {
      reason "exec blocked or failed"
    }
  }
}
"#;

/// Ungranted exec commands fail closed; granted ones run and expose
/// exit_code/stdout to the success branch.
#[test]
fn exec_is_gated_by_operator_grants() {
    let bin = env!("CARGO_BIN_EXE_whip");
    for (grant, expected) in [(None, "failed"), (Some("echo *"), "completed")] {
        let store = temp_path("exec-gate", "sqlite");
        let source = temp_path("exec-gate", "whip");
        fs::write(&source, EXEC_FLOW).expect("write source");
        let mut command = Command::new(bin);
        command.args([
            "--store",
            store.to_str().expect("utf-8"),
            "--json",
            "dev",
            source.to_str().expect("utf-8"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ]);
        match grant {
            Some(value) => {
                command.env("WHIPPLESCRIPT_EXEC_ALLOW", value);
            }
            None => {
                command.env_remove("WHIPPLESCRIPT_EXEC_ALLOW");
            }
        }
        let output = command.output().expect("command runs");
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let dev: Value =
            serde_json::from_str(&stdout[stdout.find('{').expect("json")..]).expect("dev json");
        let instance = dev
            .get("instance_id")
            .and_then(Value::as_str)
            .expect("instance id");
        assert_eq!(
            instance_status(bin, store.to_str().expect("utf-8"), instance),
            expected,
            "grant={grant:?}"
        );
        let _ = fs::remove_file(store);
        let _ = fs::remove_file(source);
    }
}

#[test]
fn hosted_check_rejects_raw_exec() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let source = temp_path("hosted-raw-exec", "whip");
    fs::write(
        &source,
        r#"
workflow HostedRawExec

output result Report

class Report {
  message string
}

rule go
  when started
=> {
  exec "echo hi" as run

  after run succeeds {
    complete result {
      message "done"
    }
  }
}
"#,
    )
    .expect("write source");
    let output = Command::new(bin)
        .args([
            "check",
            "--exec-profile",
            "hosted",
            source.to_str().expect("utf-8"),
        ])
        .output()
        .expect("command runs");
    assert!(
        !output.status.success(),
        "hosted raw exec check unexpectedly passed"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("raw `exec \"...\"` is not allowed"));
    let _ = fs::remove_file(source);
}

#[test]
fn hosted_exec_runs_content_pinned_capability_with_json_stdin() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("hosted-exec", "sqlite");
    let source = temp_path("hosted-exec", "whip");
    let script = temp_path("hosted-exec-report", "py");
    let manifest = temp_path("hosted-exec-manifest", "json");
    let script_source = "#!/usr/bin/env python3\nimport json, sys\npayload = json.load(sys.stdin)\nprint(json.dumps({\"message\": payload[\"text\"]}))\n";
    fs::write(&script, script_source).expect("write script");
    fs::write(
        &manifest,
        serde_json::json!({
            "echo_report": {
                "argv": ["python3", script.to_str().expect("utf-8")],
                "sha256": "f62856dbf1183d28f7d8f8fe8cd3aec76e66b4659e2e5f6d4b687a2d64fd1d23"
            }
        })
        .to_string(),
    )
    .expect("write manifest");
    fs::write(
        &source,
        r#"
workflow HostedExec

output result Report

class Request {
  text string
}

class Report {
  message string
}

table requests as Request [
  {
    text "hello-hosted"
  }
]

rule go
  when Request as request
=> {
  exec echo_report with request -> Report as report

  after report succeeds as out {
    complete result {
      message out.message
    }
  }
}
"#,
    )
    .expect("write source");

    let dev = dev_until_idle(
        bin,
        store.to_str().expect("utf-8"),
        source.to_str().expect("utf-8"),
        &[
            "--exec-profile",
            "hosted",
            "--script-manifest",
            manifest.to_str().expect("utf-8"),
        ],
    );
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(
        instance_status(bin, store.to_str().expect("utf-8"), instance),
        "completed"
    );
    let log = run_json(
        bin,
        &[
            "--store",
            store.to_str().expect("utf-8"),
            "--json",
            "log",
            instance,
        ],
    );
    let terminal = log
        .as_array()
        .expect("events")
        .iter()
        .find(|event| {
            event.get("event_type").and_then(Value::as_str) == Some("effect.terminal")
                && event.pointer("/payload/provider").and_then(Value::as_str) == Some("exec")
        })
        .expect("exec terminal event");
    assert_eq!(
        terminal.pointer("/payload/status").and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        terminal
            .pointer("/payload/metadata/sha256")
            .and_then(Value::as_str),
        Some("f62856dbf1183d28f7d8f8fe8cd3aec76e66b4659e2e5f6d4b687a2d64fd1d23")
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(script);
    let _ = fs::remove_file(manifest);
}

#[test]
fn hosted_exec_hash_mismatch_fails_before_spawn() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("hosted-exec-mismatch", "sqlite");
    let source = temp_path("hosted-exec-mismatch", "whip");
    let script = temp_path("hosted-exec-mismatch-report", "py");
    let marker = temp_path("hosted-exec-mismatch-marker", "txt");
    let manifest = temp_path("hosted-exec-mismatch-manifest", "json");
    let script_source = format!(
        "#!/usr/bin/env python3\nimport json, pathlib, sys\npathlib.Path({marker:?}).write_text('spawned')\npayload = json.load(sys.stdin)\nprint(json.dumps({{\"message\": payload[\"text\"]}}))\n",
        marker = marker.to_str().expect("utf-8")
    );
    fs::write(&script, script_source).expect("write script");
    fs::write(
        &manifest,
        serde_json::json!({
            "echo_report": {
                "argv": ["python3", script.to_str().expect("utf-8")],
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
            }
        })
        .to_string(),
    )
    .expect("write manifest");
    fs::write(
        &source,
        r#"
workflow HostedExecMismatch

output result Report
failure error Failed

class Request {
  text string
}

class Report {
  message string
}

class Failed {
  reason string
}

table requests as Request [
  {
    text "hello-hosted"
  }
]

rule go
  when Request as request
=> {
  exec echo_report with request -> Report as report

  after report succeeds as out {
    complete result {
      message out.message
    }
  }

  after report fails {
    fail error {
      reason "hash mismatch"
    }
  }
}
"#,
    )
    .expect("write source");

    let dev = dev_until_idle(
        bin,
        store.to_str().expect("utf-8"),
        source.to_str().expect("utf-8"),
        &[
            "--exec-profile",
            "hosted",
            "--script-manifest",
            manifest.to_str().expect("utf-8"),
        ],
    );
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(
        instance_status(bin, store.to_str().expect("utf-8"), instance),
        "failed"
    );
    assert!(!marker.exists(), "script was spawned despite hash mismatch");

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(script);
    let _ = fs::remove_file(marker);
    let _ = fs::remove_file(manifest);
}

/// Inline `decide` lowers to a coerce effect and the workflow continues from
/// its completion.
#[test]
fn inline_decide_completes() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("decide", "sqlite");
    let source = temp_path("decide", "whip");
    fs::write(
        &source,
        r#"
workflow DecideDemo

output result Verdict

class Verdict {
  summary string
}

rule go
  when started
=> {
  decide "Is this workflow worth running?" -> { worth bool, reason string } as verdict

  after verdict succeeds as v {
    complete result {
      summary "decided"
    }
  }
}
"#,
    )
    .expect("write source");
    let store_str = store.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "completed");
    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// Unknown statements and malformed modifiers are check errors, not silent
/// no-ops.
#[test]
fn unknown_statements_and_modifiers_are_rejected() {
    let bin = env!("CARGO_BIN_EXE_whip");
    for (label, body, expected) in [
        (
            "unknown-stmt",
            "frobnicate task",
            "unknown rule body statement `frobnicate`",
        ),
        (
            "bad-timeout",
            "tell worker as turn timeout banana \"go\"",
            "expected a duration after `timeout`",
        ),
        (
            "flow-only",
            "on fails {\n    record Seen {\n      id \"x\"\n    }\n  }",
            "only valid inside `flow` bodies",
        ),
    ] {
        let source = temp_path(label, "whip");
        fs::write(
            &source,
            format!(
                r#"
workflow Gate

class Seen {{
  id string
}}

agent worker {{
  provider fixture
  profile "x"
  capacity 1
}}

@external
rule go
  when Seen as task
=> {{
  {body}
}}
"#
            ),
        )
        .expect("write source");
        let output = Command::new(bin)
            .args(["check", source.to_str().expect("utf-8")])
            .output()
            .expect("command runs");
        assert!(!output.status.success(), "{label} should fail check");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{label}: {stderr}");
        let _ = fs::remove_file(source);
    }
}

const TIMER_UNTIL_SOURCE: &str = r#"
workflow TimerUntil

output result Done

class Done {
  note string
}

class Job {
  id string
  fireAt time
}

rule seed
  when started
=> {
  record Job {
    id "J-1"
    fireAt "DEADLINE"
  }
}

rule wait_for_deadline
  when Job as j
=> {
  timer until OPERAND as deadline

  after deadline succeeds {
    complete result {
      note "deadline reached"
    }
  }
}
"#;

fn timer_until_source(deadline: &str, operand: &str) -> String {
    TIMER_UNTIL_SOURCE
        .replace("DEADLINE", deadline)
        .replace("OPERAND", operand)
}

/// An absolute `timer until` literal in the past fires on the worker pass and
/// the `after ... succeeds` terminal completes the instance — from a fact
/// trigger, the case where the deadline comparison and NULL duration mapping
/// previously broke the time pass.
#[test]
fn timer_until_past_literal_completes_from_fact_trigger() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("until-past", "sqlite");
    let source = temp_path("until-past", "whip");
    fs::write(
        &source,
        timer_until_source("2020-01-01T00:00:00Z", "\"2020-01-01T00:00:00Z\""),
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "completed");

    let effects = run_json(bin, &["--store", store_str, "--json", "effects", instance]);
    let timer = effects
        .as_array()
        .expect("effects")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("timer.wait"))
        .expect("timer effect");
    assert_eq!(
        timer.get("status").and_then(Value::as_str),
        Some("completed")
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// `timer until <path>` resolves a `time`-typed fact field into the effect's
/// absolute deadline and fires identically to the literal form.
#[test]
fn timer_until_fact_path_resolves_and_completes() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("until-path", "sqlite");
    let source = temp_path("until-path", "whip");
    fs::write(
        &source,
        timer_until_source("2020-01-01T00:00:00Z", "j.fireAt"),
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "completed");

    let effects = run_json(bin, &["--store", store_str, "--json", "effects", instance]);
    let timer = effects
        .as_array()
        .expect("effects")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("timer.wait"))
        .expect("timer effect");
    assert_eq!(
        timer.pointer("/input/deadline_at").and_then(Value::as_str),
        Some("2020-01-01T00:00:00Z"),
        "path operand resolves to the recorded instant, not the path text"
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// A future absolute deadline pends: the timer stays queued and the instance
/// stays running until the clock reaches the target.
#[test]
fn timer_until_future_deadline_stays_queued() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("until-future", "sqlite");
    let source = temp_path("until-future", "whip");
    fs::write(
        &source,
        timer_until_source("2099-01-01T00:00:00Z", "j.fireAt"),
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "running");

    let effects = run_json(bin, &["--store", store_str, "--json", "effects", instance]);
    let timer = effects
        .as_array()
        .expect("effects")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("timer.wait"))
        .expect("timer effect");
    assert_eq!(timer.get("status").and_then(Value::as_str), Some("queued"));

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

fn dev_with_exec_allow(bin: &str, store: &str, source: &str, allow: &str) -> Value {
    let mut command = Command::new(bin);
    command
        .args([
            "--store",
            store,
            "--json",
            "dev",
            source,
            "--provider",
            "fixture",
            "--until",
            "idle",
        ])
        .env("WHIPPLESCRIPT_EXEC_ALLOW", allow);
    let output = command.output().expect("command runs");
    assert!(
        output.status.success(),
        "dev failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout[stdout.find('{').expect("json")..]).expect("dev json")
}

const EXEC_PARSE_SINGLE: &str = r#"
workflow ExecParseSingle

output result Done
failure error Failed

class Done {
  total int
}

class Failed {
  reason string
}

class Report {
  count int
  label string
}

rule run_report
  when started
=> {
  exec "COMMAND" -> Report as x

  after x succeeds as r {
    complete result {
      total r.count
    }
  }
  after x fails {
    fail error {
      reason "report did not parse"
    }
  }
}
"#;

/// `exec ... -> Report as x` parses stdout at the result boundary and binds
/// the typed value in the success branch — including escaped quotes in the
/// command string.
#[test]
fn exec_parse_single_binds_typed_value() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("exec-parse", "sqlite");
    let source = temp_path("exec-parse", "whip");
    fs::write(
        &source,
        EXEC_PARSE_SINGLE.replace(
            "COMMAND",
            "echo '{\\\"count\\\": 7, \\\"label\\\": \\\"ok\\\"}'",
        ),
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_with_exec_allow(bin, store_str, source.to_str().expect("utf-8"), "echo *");
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "completed");

    let trace = run_json(bin, &["--store", store_str, "--json", "status", instance]);
    assert_eq!(
        trace
            .pointer("/workflow_terminal/payload/total")
            .and_then(Value::as_i64),
        Some(7),
        "{trace}"
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// Parse or schema-validation failure makes the effect fail even on exit 0,
/// routing to `after x fails`.
#[test]
fn exec_parse_failure_routes_to_fails_branch() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("exec-parse-bad", "sqlite");
    let source = temp_path("exec-parse-bad", "whip");
    fs::write(
        &source,
        EXEC_PARSE_SINGLE.replace("COMMAND", "echo not-json"),
    )
    .expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_with_exec_allow(bin, store_str, source.to_str().expect("utf-8"), "echo *");
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "failed");

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

const EXEC_PARSE_EACH: &str = r#"
workflow ExecParseEach

output result Done

class Done {
  total int
}

class WorkItem {
  id string
  title string
}

rule list_items
  when started
=> {
  exec "COMMAND" -> each WorkItem
}

rule finish
  when WorkItem as item where count(WorkItem) == 3
=> {
  complete result {
    total count(WorkItem)
  }
}
"#;

/// `-> each` records one typed `ingest` fact per JSONL line and ordinary
/// per-fact rule fan-out reacts to them.
#[test]
fn exec_parse_each_fans_out_ingest_facts() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("exec-each", "sqlite");
    let source = temp_path("exec-each", "whip");
    let script = temp_path("exec-each", "sh");
    fs::write(
        &script,
        "#!/bin/sh\necho '{\"id\": \"W-1\", \"title\": \"first\"}'\necho '{\"id\": \"W-2\", \"title\": \"second\"}'\necho '{\"id\": \"W-3\", \"title\": \"third\"}'\n",
    )
    .expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let script_str = script.to_str().expect("utf-8");
    fs::write(&source, EXEC_PARSE_EACH.replace("COMMAND", script_str)).expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_with_exec_allow(bin, store_str, source.to_str().expect("utf-8"), script_str);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "completed");

    let facts = run_json(bin, &["--store", store_str, "--json", "facts", instance]);
    let items = facts
        .as_array()
        .expect("facts")
        .iter()
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("WorkItem"))
        .collect::<Vec<_>>();
    assert_eq!(items.len(), 3, "{facts}");
    for item in items {
        assert_eq!(
            item.get("provenance_class").and_then(Value::as_str),
            Some("ingest"),
            "{item}"
        );
    }

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(script);
}

/// A malformed line fails the whole stream: the effect lands `failed` and no
/// partial facts commit.
#[test]
fn exec_parse_stream_is_all_or_nothing() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("exec-atomic", "sqlite");
    let source = temp_path("exec-atomic", "whip");
    let script = temp_path("exec-atomic", "sh");
    fs::write(
        &script,
        "#!/bin/sh\necho '{\"id\": \"W-1\", \"title\": \"first\"}'\necho 'not json at all'\necho '{\"id\": \"W-3\", \"title\": \"third\"}'\n",
    )
    .expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let script_str = script.to_str().expect("utf-8");
    fs::write(&source, EXEC_PARSE_EACH.replace("COMMAND", script_str)).expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_with_exec_allow(bin, store_str, source.to_str().expect("utf-8"), script_str);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    let effects = run_json(bin, &["--store", store_str, "--json", "effects", instance]);
    let exec = effects
        .as_array()
        .expect("effects")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("exec.command"))
        .expect("exec effect");
    assert_eq!(exec.get("status").and_then(Value::as_str), Some("failed"));

    let facts = run_json(bin, &["--store", store_str, "--json", "facts", instance]);
    let items = facts
        .as_array()
        .expect("facts")
        .iter()
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("WorkItem"))
        .count();
    assert_eq!(items, 0, "partial stream must not commit");

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(script);
}

/// `->` parse targets are statically checked: schema must exist, single needs
/// a binding, `each` rejects one.
#[test]
fn exec_parse_static_checks_reject_bad_targets() {
    let bin = env!("CARGO_BIN_EXE_whip");
    for (label, source_text, expected) in [
        (
            "unknown-schema",
            EXEC_PARSE_SINGLE
                .replace("COMMAND", "echo x")
                .replace("-> Report as x", "-> Mystery as x"),
            "unknown schema `Mystery`",
        ),
        (
            "single-without-binding",
            EXEC_PARSE_SINGLE
                .replace("COMMAND", "echo x")
                .replace("-> Report as x", "-> Report"),
            "needs an `as` binding",
        ),
        (
            "each-with-binding",
            EXEC_PARSE_EACH
                .replace("COMMAND", "echo x")
                .replace("-> each WorkItem", "-> each WorkItem as items"),
            "stream of facts, not a single binding",
        ),
    ] {
        let source = temp_path(label, "whip");
        fs::write(&source, source_text).expect("write source");
        let output = Command::new(bin)
            .args(["check", source.to_str().expect("utf-8")])
            .output()
            .expect("command runs");
        assert!(!output.status.success(), "{label} should fail check");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{label}: {stderr}");
        let _ = fs::remove_file(source);
    }
}

const LEASE_SOURCE: &str = r#"
workflow LeaseDemo

output result Done
failure error Failed

class Done {
  note string
}

class Failed {
  reason string
}

class Environment {
  name string
}

class ReadyToShip {
  env string
}

lease deploy_slot {
  key Environment
  slots 1
  ttl 10m
}

rule seed
  when started
=> {
  record ReadyToShip {
    env "prod"
  }
}

rule ship
  when ReadyToShip as r
=> {
  acquire deploy_slot for r.env as slot

  after slot held {
    release slot
    complete result {
      note "deployed {{ r.env }}"
    }
  }
  after slot contended {
    fail error {
      reason "deploy slot busy"
    }
  }
}
"#;

fn dev_with_coordination(bin: &str, store: &str, source: &str, coordination: &str) -> Value {
    let mut command = Command::new(bin);
    command
        .args([
            "--store",
            store,
            "--json",
            "dev",
            source,
            "--provider",
            "fixture",
            "--until",
            "idle",
        ])
        .env("WHIPPLESCRIPT_COORDINATION_STORE", coordination);
    let output = command.output().expect("command runs");
    assert!(
        output.status.success(),
        "dev failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout[stdout.find('{').expect("json")..]).expect("dev json")
}

/// Acquire wins a free slot, the `held` arm runs, and explicit release plus
/// terminal auto-release leave no lease behind.
#[test]
fn lease_acquire_held_release_completes() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("lease-held", "sqlite");
    let source = temp_path("lease-held", "whip");
    let coordination = temp_path("lease-held-coord", "sqlite");
    fs::write(&source, LEASE_SOURCE).expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let coordination_str = coordination.to_str().expect("utf-8");
    let dev = dev_with_coordination(
        bin,
        store_str,
        source.to_str().expect("utf-8"),
        coordination_str,
    );
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "completed");

    let leases = {
        let output = Command::new(bin)
            .args(["--json", "leases"])
            .env("WHIPPLESCRIPT_COORDINATION_STORE", coordination_str)
            .output()
            .expect("command runs");
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str::<Value>(&stdout[stdout.find('[').expect("json")..]).expect("json")
    };
    assert_eq!(leases.as_array().map(Vec::len), Some(0), "{leases}");

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(coordination);
}

/// A held slot contends: the second instance routes to the `contended` arm —
/// a normal branchable outcome, not an error.
#[test]
fn lease_contention_routes_to_contended_arm() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("lease-contended", "sqlite");
    let source = temp_path("lease-contended", "whip");
    let coordination = temp_path("lease-contended-coord", "sqlite");
    fs::write(&source, LEASE_SOURCE).expect("write source");

    // Seed a foreign holder so the acquire contends.
    {
        let mut seeded = whipplescript_store::coordination::CoordinationStore::open(&coordination)
            .expect("open coordination store");
        assert_eq!(
            seeded
                .try_acquire("deploy_slot", "prod", 1, 600, "ins_other")
                .expect("seed holder"),
            whipplescript_store::coordination::AcquireOutcome::Held
        );
    }

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_with_coordination(
        bin,
        store_str,
        source.to_str().expect("utf-8"),
        coordination.to_str().expect("utf-8"),
    );
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    assert_eq!(instance_status(bin, store_str, instance), "failed");

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(coordination);
}

const COUNTER_SOURCE: &str = r#"
workflow CounterDemo

output result Done
failure error Failed

class Done {
  note string
}

class Failed {
  reason string
}

class Customer {
  id string
}

class ReviewTask {
  customer string
  estTokens int
}

counter model_budget {
  key Customer
  cap 1000
  reset daily
}

rule seed
  when started
=> {
  record ReviewTask {
    customer "cust-1"
    estTokens 600
  }
}

rule review
  when ReviewTask as t
=> {
  consume model_budget for t.customer amount t.estTokens as spend

  after spend ok {
    complete result {
      note "within budget"
    }
  }
  after spend over {
    fail error {
      reason "over budget"
    }
  }
}
"#;

/// Cap invariant across instances: the first consume fits, the second
/// (sharing the workspace counter) is over budget and downgrades.
#[test]
fn counter_consume_ok_then_over() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let coordination = temp_path("counter-coord", "sqlite");
    let coordination_str = coordination.to_str().expect("utf-8");
    let source = temp_path("counter", "whip");
    fs::write(&source, COUNTER_SOURCE).expect("write source");

    for (label, expected) in [("first", "completed"), ("second", "failed")] {
        let store = temp_path(&format!("counter-{label}"), "sqlite");
        let store_str = store.to_str().expect("utf-8");
        let dev = dev_with_coordination(
            bin,
            store_str,
            source.to_str().expect("utf-8"),
            coordination_str,
        );
        let instance = dev
            .get("instance_id")
            .and_then(Value::as_str)
            .expect("instance id");
        assert_eq!(
            instance_status(bin, store_str, instance),
            expected,
            "{label}"
        );
        let _ = fs::remove_file(store);
    }

    let _ = fs::remove_file(source);
    let _ = fs::remove_file(coordination);
}

/// The coordination safety model is enforced statically: undeclared
/// resources, unhandled outcomes, leak-prone held branches, and multi-lease
/// progressions are check errors.
#[test]
fn coordination_static_checks() {
    let bin = env!("CARGO_BIN_EXE_whip");
    for (label, source_text, expected) in [
        (
            "undeclared-lease",
            LEASE_SOURCE.replace("lease deploy_slot", "lease other_slot"),
            "acquires undeclared lease `deploy_slot`",
        ),
        (
            "missing-contended",
            LEASE_SOURCE.replace(
                "  after slot contended {\n    fail error {\n      reason \"deploy slot busy\"\n    }\n  }\n",
                "",
            ),
            "does not handle the `contended` outcome",
        ),
        (
            "held-forever",
            LEASE_SOURCE.replace(
                "  after slot held {\n    release slot\n    complete result {\n      note \"deployed {{ r.env }}\"\n    }\n  }\n",
                "  after slot held {\n    record ReadyToShip {\n      env \"staging\"\n    }\n  }\n",
            ),
            "can hold lease `slot` forever",
        ),
        (
            "two-leases",
            LEASE_SOURCE.replace(
                "  acquire deploy_slot for r.env as slot\n",
                "  acquire deploy_slot for r.env as slot\n  acquire deploy_slot for r.env as slot2\n",
            ),
            "acquires more than one lease in a single progression",
        ),
        (
            "missing-over",
            COUNTER_SOURCE.replace(
                "  after spend over {\n    fail error {\n      reason \"over budget\"\n    }\n  }\n",
                "",
            ),
            "does not handle the `over` outcome",
        ),
    ] {
        let source = temp_path(label, "whip");
        fs::write(&source, source_text).expect("write source");
        let output = Command::new(bin)
            .args(["check", source.to_str().expect("utf-8")])
            .output()
            .expect("command runs");
        assert!(!output.status.success(), "{label} should fail check");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{label}: {stderr}");
        let _ = fs::remove_file(source);
    }
}

/// The OTel ambassador (spec/observability.md C8): `whip otel-export` builds
/// OTLP-JSON from the durable log; `--dry-run` prints it, and structural
/// attributes (never content) ride each span.
#[test]
fn otel_export_dry_run_emits_structural_spans() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("otel", "sqlite");
    let source = temp_path("otel", "whip");
    let coordination = temp_path("otel-coord", "sqlite");
    fs::write(&source, LEASE_SOURCE).expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let dev = dev_with_coordination(
        bin,
        store_str,
        source.to_str().expect("utf-8"),
        coordination.to_str().expect("utf-8"),
    );
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    let payload = run_json(
        bin,
        &["--store", store_str, "otel-export", instance, "--dry-run"],
    );
    let spans = payload
        .pointer("/resourceSpans/0/scopeSpans/0/spans")
        .and_then(Value::as_array)
        .expect("spans");
    assert!(!spans.is_empty(), "{payload}");
    let span = &spans[0];
    assert!(
        span.get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.contains("lease.acquire")),
        "{span}"
    );
    let attributes = span
        .get("attributes")
        .and_then(Value::as_array)
        .expect("attrs");
    assert!(attributes.iter().any(|attr| {
        attr.get("key").and_then(Value::as_str) == Some("whipplescript.instance_id")
    }));

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(coordination);
}

/// `whip otel-export` POSTs OTLP/HTTP+JSON to a real endpoint (validated here by
/// an in-process collector — no external backend needed), then `whip telemetry
/// status` reflects the emit-once cursor and `whip telemetry reset-cursor` clears
/// it (spec/std-telemetry.md export-cursor management).
#[test]
fn otel_export_posts_to_collector_then_status_and_reset() {
    use std::io::{Read, Write};

    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("otel-post", "sqlite");
    let source = temp_path("otel-post", "whip");
    let coordination = temp_path("otel-post-coord", "sqlite");
    fs::write(&source, LEASE_SOURCE).expect("write source");
    let store_str = store.to_str().expect("utf-8").to_owned();
    let dev = dev_with_coordination(
        bin,
        &store_str,
        source.to_str().expect("utf-8"),
        coordination.to_str().expect("utf-8"),
    );
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();

    // In-process OTLP collector: accept one POST, capture its body, reply 200.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind collector");
    let port = listener.local_addr().expect("addr").port();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let collector = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let read = stream.read(&mut chunk).unwrap_or(0);
                if read == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..read]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                    let content_length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let body_start = pos + 4;
                    while buf.len() < body_start + content_length {
                        let read = stream.read(&mut chunk).unwrap_or(0);
                        if read == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..read]);
                    }
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                    let body = String::from_utf8_lossy(&buf[body_start..]).to_string();
                    let _ = tx.send(body);
                    break;
                }
            }
        }
    });

    let export = Command::new(bin)
        .args(["--store", &store_str, "otel-export", &instance])
        .env(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            format!("http://127.0.0.1:{port}"),
        )
        .env("OTEL_SERVICE_NAME", "whip-test")
        .output()
        .expect("otel-export runs");
    assert!(
        export.status.success(),
        "otel-export failed: {}",
        String::from_utf8_lossy(&export.stderr)
    );

    let body = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("collector received the OTLP POST");
    collector.join().ok();
    let received: Value = serde_json::from_str(&body).expect("collector body is OTLP JSON");
    assert!(
        received
            .pointer("/resourceSpans/0/scopeSpans/0/spans")
            .and_then(Value::as_array)
            .is_some_and(|spans| !spans.is_empty()),
        "expected exported spans in the OTLP payload: {body}"
    );

    // `telemetry status` reflects the emit-once cursor.
    let status = run_json(
        bin,
        &["--store", &store_str, "--json", "telemetry", "status"],
    );
    assert!(
        status
            .pointer("/instances/0/exported_runs")
            .and_then(Value::as_u64)
            .is_some_and(|count| count >= 1),
        "status should show exported runs: {status}"
    );

    // `telemetry reset-cursor` clears it; a follow-up status shows nothing.
    let reset = run_json(
        bin,
        &["--store", &store_str, "--json", "telemetry", "reset-cursor"],
    );
    assert_eq!(
        reset.get("cleared").and_then(Value::as_bool),
        Some(true),
        "{reset}"
    );
    let status_after = run_json(
        bin,
        &["--store", &store_str, "--json", "telemetry", "status"],
    );
    assert!(
        status_after
            .pointer("/instances")
            .and_then(Value::as_array)
            .is_some_and(|instances| instances.is_empty()),
        "cursor should be cleared after reset: {status_after}"
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
    let _ = fs::remove_file(coordination);
}

const EVENT_SOURCE: &str = r#"
workflow EventDemo

output result Done

class Done {
  note string
}

signal deploy.finished {
  service string
  status string
}

rule on_deploy
  when deploy.finished as d
=> {
  complete result {
    note "deploy of {{ d.service }}: {{ d.status }}"
  }
}
"#;

/// `whip signal` validates the payload against the declared `signal` schema,
/// lands a typed durable fact, and the bare `when <signal> as d` reaction
/// fires — no `@external` tag needed.
#[test]
fn signal_lands_typed_fact_and_rule_fires() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("signal", "sqlite");
    let source = temp_path("signal", "whip");
    fs::write(&source, EVENT_SOURCE).expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let source_str = source.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source_str, &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();
    assert_eq!(instance_status(bin, store_str, &instance), "running");

    let signaled = run_json(
        bin,
        &[
            "--store",
            store_str,
            "--json",
            "signal",
            &instance,
            "--name",
            "deploy.finished",
            "--data",
            r#"{"service":"api","status":"ok"}"#,
            "--program",
            source_str,
        ],
    );
    assert_eq!(
        signaled.get("signal").and_then(Value::as_str),
        Some("deploy.finished")
    );
    run_text(
        bin,
        &[
            "--store",
            store_str,
            "step",
            &instance,
            "--program",
            source_str,
        ],
    );
    assert_eq!(instance_status(bin, store_str, &instance), "completed");

    let status = run_json(bin, &["--store", store_str, "--json", "status", &instance]);
    assert_eq!(
        status
            .pointer("/workflow_terminal/payload/note")
            .and_then(Value::as_str),
        Some("deploy of api: ok")
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// A malformed payload or undeclared signal is rejected at the CLI boundary;
/// no ill-typed fact can land.
#[test]
fn signal_rejects_bad_payload_and_unknown_signal() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_path("signal-reject", "sqlite");
    let source = temp_path("signal-reject", "whip");
    fs::write(&source, EVENT_SOURCE).expect("write source");

    let store_str = store.to_str().expect("utf-8");
    let source_str = source.to_str().expect("utf-8");
    let dev = dev_until_idle(bin, store_str, source_str, &[]);
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();

    for (label, signal, data, expected) in [
        (
            "bad-payload",
            "deploy.finished",
            r#"{"service":"api","bogus":1}"#,
            "does not conform to signal",
        ),
        ("unknown-signal", "deploy.nope", "{}", "is not declared"),
    ] {
        let output = Command::new(bin)
            .args([
                "--store",
                store_str,
                "signal",
                &instance,
                "--name",
                signal,
                "--data",
                data,
                "--program",
                source_str,
            ])
            .output()
            .expect("command runs");
        assert!(!output.status.success(), "{label} should be rejected");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{label}: {stderr}");
    }
    assert_eq!(instance_status(bin, store_str, &instance), "running");

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(source);
}

/// Signal reactions are statically checked: the bare `when` form requires a
/// declared signal, and field access on the binding is typed against the
/// signal schema.
#[test]
fn signal_static_checks() {
    let bin = env!("CARGO_BIN_EXE_whip");
    for (label, source_text, expected) in [
        (
            "undeclared-signal",
            EVENT_SOURCE.replace("when deploy.finished as d", "when deploy.unknown as d"),
            "reacts to undeclared signal `deploy.unknown`",
        ),
        (
            "bad-event-field",
            EVENT_SOURCE.replace("{{ d.status }}", "{{ d.bogus }}"),
            "schema `deploy.finished` has no field `bogus`",
        ),
    ] {
        let source = temp_path(label, "whip");
        fs::write(&source, source_text).expect("write source");
        let output = Command::new(bin)
            .args(["check", source.to_str().expect("utf-8")])
            .output()
            .expect("command runs");
        assert!(!output.status.success(), "{label} should fail check");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{label}: {stderr}");
        let _ = fs::remove_file(source);
    }
}

const SUM_TYPE_TABLE: &str = r#"
workflow SumTable

output result Done
failure error Failed

class Done {
  note string
}

class Failed {
  reason string
}

enum ReviewOutcome {
  Approved {
    score float
  }
  Rejected {
    reason string
  }
  Blocked
}

class Review {
  id string
  outcome ReviewOutcome
}

table reviews as Review [
  { id "R-1" outcome SEED }
]

rule judge
  when Review as r
=> {
  case r.outcome {
    Approved as a => {
      complete result {
        note "approved with {{ a.score }}"
      }
    }
    Rejected as rej => {
      fail error {
        reason rej.reason
      }
    }
    Blocked => {
      fail error {
        reason "blocked"
      }
    }
  }
}
"#;

/// A data-carrying variant constructed in a table row lands as an
/// internally-tagged record; `case` dispatches on the synthesized `variant`
/// discriminant and `as` binds the payload.
#[test]
fn sum_type_case_dispatches_and_binds_payload() {
    let bin = env!("CARGO_BIN_EXE_whip");
    for (label, seed, expected_status, expected_payload) in [
        (
            "approved",
            "Approved { score 0.9 }",
            "completed",
            r#""note":"approved with 0.9""#,
        ),
        (
            "rejected",
            r#"Rejected { reason "too vague" }"#,
            "failed",
            r#""reason":"too vague""#,
        ),
        ("bare", "Blocked", "failed", r#""reason":"blocked""#),
    ] {
        let store = temp_path(&format!("sum-{label}"), "sqlite");
        let source = temp_path(&format!("sum-{label}"), "whip");
        fs::write(&source, SUM_TYPE_TABLE.replace("SEED", seed)).expect("write source");

        let store_str = store.to_str().expect("utf-8");
        let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), &[]);
        let instance = dev
            .get("instance_id")
            .and_then(Value::as_str)
            .expect("instance id");
        assert_eq!(
            instance_status(bin, store_str, instance),
            expected_status,
            "{label}"
        );
        let status = run_json(bin, &["--store", store_str, "--json", "status", instance]);
        let payload = status
            .pointer("/workflow_terminal/payload")
            .map(Value::to_string)
            .unwrap_or_default();
        assert!(payload.contains(expected_payload), "{label}: {payload}");

        let _ = fs::remove_file(store);
        let _ = fs::remove_file(source);
    }
}

const SUM_TYPE_COERCE: &str = r#"
workflow SumCoerce

output result Done
failure error Failed

class Done {
  note string
}

class Failed {
  reason string
}

enum ReviewOutcome {
  Approved {
    score float
  }
  Rejected {
    reason string
  }
}

class Draft {
  body string
}

coerce reviewDraft(draft string) -> ReviewOutcome {
  prompt """
  Review this draft: {{ draft }}
  """
}

rule seed
  when started
=> {
  record Draft {
    body "the draft"
  }
}

rule review
  when Draft as d
=> {
  coerce reviewDraft(d.body) as outcome

  after outcome succeeds as o {
    case o {
      Approved as a => {
        complete result {
          note "score {{ a.score }}"
        }
      }
      Rejected as rej => {
        fail error {
          reason rej.reason
        }
      }
    }
  }
}
"#;

/// A fixture coerce over a sum type returns the first declared variant as a
/// tagged record by default; `--variant` selects another arm to exercise its
/// case branch without a real provider.
#[test]
fn sum_type_coerce_fixture_selects_variants() {
    let bin = env!("CARGO_BIN_EXE_whip");
    for (extra, expected_status) in [
        (None, "completed"),
        (Some(["--variant", "Rejected"]), "failed"),
    ] {
        let store = temp_path("sum-coerce", "sqlite");
        let source = temp_path("sum-coerce", "whip");
        fs::write(&source, SUM_TYPE_COERCE).expect("write source");

        let store_str = store.to_str().expect("utf-8");
        let extra_args = extra.as_ref().map(|args| &args[..]).unwrap_or(&[]);
        let dev = dev_until_idle(bin, store_str, source.to_str().expect("utf-8"), extra_args);
        let instance = dev
            .get("instance_id")
            .and_then(Value::as_str)
            .expect("instance id");
        assert_eq!(
            instance_status(bin, store_str, instance),
            expected_status,
            "extra={extra:?}"
        );

        let _ = fs::remove_file(store);
        let _ = fs::remove_file(source);
    }
}

/// Sum-type declarations and case blocks are statically checked: the
/// discriminant name is reserved, bare variants take no binding, and case
/// stays exhaustive over the variant set.
#[test]
fn sum_type_static_checks() {
    let bin = env!("CARGO_BIN_EXE_whip");
    for (label, source_text, expected) in [
        (
            "reserved-variant-field",
            SUM_TYPE_TABLE
                .replace("SEED", "Blocked")
                .replace("score float", "variant string"),
            "declares reserved field `variant`",
        ),
        (
            "binding-on-bare-variant",
            SUM_TYPE_TABLE
                .replace("SEED", "Blocked")
                .replace("Blocked => {", "Blocked as b => {"),
            "carries no payload to bind",
        ),
        (
            "non-exhaustive",
            SUM_TYPE_TABLE.replace("SEED", "Blocked").replace(
                "    Blocked => {\n      fail error {\n        reason \"blocked\"\n      }\n    }\n",
                "",
            ),
            "non-exhaustive case; missing Blocked",
        ),
    ] {
        let source = temp_path(label, "whip");
        fs::write(&source, source_text).expect("write source");
        let output = Command::new(bin)
            .args(["check", source.to_str().expect("utf-8")])
            .output()
            .expect("command runs");
        assert!(!output.status.success(), "{label} should fail check");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{label}: {stderr}");
        let _ = fs::remove_file(source);
    }
}

/// `timer until` operands are statically checked: literals must be ISO-8601
/// instants and paths must resolve to `time`-typed fields.
#[test]
fn timer_until_static_checks_reject_bad_operands() {
    let bin = env!("CARGO_BIN_EXE_whip");
    for (label, deadline, operand, expected) in [
        (
            "bad-literal",
            "2020-01-01T00:00:00Z",
            "\"next tuesday\"",
            "invalid time literal `next tuesday`",
        ),
        (
            "non-time-path",
            "2020-01-01T00:00:00Z",
            "j.id",
            "non-time operand `j.id` in `timer until`",
        ),
        (
            "unknown-binding",
            "2020-01-01T00:00:00Z",
            "ghost.fireAt",
            "unknown binding `ghost` in `timer until ghost.fireAt`",
        ),
    ] {
        let source = temp_path(label, "whip");
        fs::write(&source, timer_until_source(deadline, operand)).expect("write source");
        let output = Command::new(bin)
            .args(["check", source.to_str().expect("utf-8")])
            .output()
            .expect("command runs");
        assert!(!output.status.success(), "{label} should fail check");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{label}: {stderr}");
        let _ = fs::remove_file(source);
    }
}

/// DR-0025 cross-package `@tool`: an owned-harness agent granted a workflow tool
/// exported by a `use`d package resolves it from the package attestation, drives
/// the package's shipped source synchronously, and the parent turn completes. The
/// example program/lock/package are the committed fixtures. The parent's
/// `after turn succeeds` only fires if the granted tool resolved and the turn
/// succeeded, so a completed parent proves the cross-package grant ran end to end.
#[test]
fn cross_package_tool_grant_resolves_and_runs() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let consumer = examples.join("subworkflow-tool-consumer.whip");
    let lock = examples.join("subworkflow-tool-consumer.lock.json");
    let store = temp_path("xpkg-tool", "sqlite");
    let workspace = temp_path("xpkg-tool-ws", "dir");
    let coordination = temp_path("xpkg-tool-coord", "sqlite");
    fs::create_dir_all(&workspace).expect("workspace dir");

    let output = Command::new(bin)
        .args([
            "--store",
            store.to_str().expect("utf-8"),
            "--json",
            "dev",
            consumer.to_str().expect("utf-8"),
            "--package-lock",
            lock.to_str().expect("utf-8"),
            "--root",
            "ConsumerFlow",
            "--provider",
            "owned",
            "--input",
            r#"{"request":{"task":"echo it"}}"#,
            "--until",
            "idle",
        ])
        .env(
            "WHIPPLESCRIPT_OWNED_FIXTURE_TOOL",
            r#"EchoText:{"request":{"text":"cross-package regression"}}"#,
        )
        .env("WHIPPLESCRIPT_HARNESS_WORKSPACE", &workspace)
        .env("WHIPPLESCRIPT_COORDINATION_STORE", &coordination)
        .output()
        .expect("command runs");
    assert!(
        output.status.success(),
        "dev failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    let json_start = stdout.find(['{', '[']).expect("json output");
    let dev: Value = serde_json::from_str(&stdout[json_start..]).expect("valid json");
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");
    let store_str = store.to_str().expect("utf-8");
    assert_eq!(
        instance_status(bin, store_str, instance),
        "completed",
        "consumer flow did not complete: {dev}"
    );
    // The brokered turn requested and received a result for the granted package
    // tool — the cross-package invoke path actually ran, not just type-checked.
    let evidence_kinds = dev
        .pointer("/provider_evidence/groups")
        .and_then(Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .filter_map(|group| group.get("kind").and_then(Value::as_str))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        evidence_kinds
            .iter()
            .any(|kind| kind == "agent.turn.brokered.tool_result"),
        "expected a brokered tool result from the package tool: {evidence_kinds:?}"
    );

    let _ = fs::remove_file(store);
    let _ = fs::remove_file(coordination);
    let _ = fs::remove_dir_all(workspace);
}

const IFC_BAD_WHIP: &str = r#"@service
workflow IfcCheck

output result R
class R { ok bool }
class Ticket { id string  status "open" }

agent coder { provider fixture  profile "repo-writer"  capacity 1 }

file store ledger { root "./ledger"  allow read ["**"] }
file store outbox { root "./outbox"  allow write ["**"] }

table seed as Ticket [ { id "T1"  status "open" } ]

rule work
  when Ticket as ticket where ticket.status == "open"
  when coder is available
=> {
  tell coder as turn
    with access to ledger {
      read ["**"]
    }
    with access to outbox {
      write ["**"]
    }
  "go"

  after turn succeeds as outcome {
    complete result { ok true }
  }
}
"#;

const IFC_ENVELOPE: &str = r#"{ "resources": {
  "ledger": { "confidential": true },
  "outbox": { "confidential": false }
} }"#;

/// End-to-end: with a governance envelope a turn that reads a confidential
/// resource and writes an un-cleared one is rejected by `whip check`; without the
/// envelope the same whip passes (the gradual / dev mode).
#[test]
fn ifc_check_rejects_confidential_to_uncleared_flow_under_envelope() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let whip = temp_path("ifc-whip", "whip");
    let envelope = temp_path("ifc-env", "json");
    fs::write(&whip, IFC_BAD_WHIP).expect("write whip");
    fs::write(&envelope, IFC_ENVELOPE).expect("write envelope");

    let governed = Command::new(bin)
        .args(["check", whip.to_str().expect("utf-8 path")])
        .env("WHIPPLESCRIPT_IFC_ENVELOPE", &envelope)
        .output()
        .expect("command runs");
    let governed_stderr = String::from_utf8_lossy(&governed.stderr);
    assert!(
        !governed.status.success(),
        "expected check to fail under governance\nstderr:\n{governed_stderr}"
    );
    assert!(
        governed_stderr.contains("information-flow violation"),
        "expected an IFC violation\nstderr:\n{governed_stderr}"
    );

    let dev = Command::new(bin)
        .args(["check", whip.to_str().expect("utf-8 path")])
        .env_remove("WHIPPLESCRIPT_IFC_ENVELOPE")
        .output()
        .expect("command runs");
    assert!(
        dev.status.success(),
        "expected dev-mode check to pass (no envelope)\nstderr:\n{}",
        String::from_utf8_lossy(&dev.stderr)
    );

    let _ = fs::remove_file(&whip);
    let _ = fs::remove_file(&envelope);
}

/// End-to-end two-agent separation (DR-0028 D5): `whip gov sign` is refused
/// without governance privilege (the whip agent) and succeeds with it (the
/// governance agent); the resulting signed envelope verifies unprivileged.
#[test]
fn ifc_two_agent_sign_requires_governance_privilege() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let config = temp_path("gov-config", "gov");
    fs::write(
        &config,
        "grant file_store ledger -> file:/srv/ledger.db readable by Operator\n",
    )
    .expect("write config");
    let config_arg = config.to_str().expect("utf-8 path");

    // whip agent (no privilege): sign refused (G4)
    let unprivileged = Command::new(bin)
        .args(["gov", "sign", config_arg])
        .env_remove("WHIPPLESCRIPT_GOV_ADMIN")
        .output()
        .expect("command runs");
    assert!(
        !unprivileged.status.success(),
        "unprivileged sign must be refused"
    );
    assert!(
        String::from_utf8_lossy(&unprivileged.stderr).contains("privilege"),
        "stderr: {}",
        String::from_utf8_lossy(&unprivileged.stderr)
    );

    // governance agent (privileged, sudo proxy): sign succeeds
    let signed = Command::new(bin)
        .args(["gov", "sign", config_arg])
        .env("WHIPPLESCRIPT_GOV_ADMIN", "1")
        .output()
        .expect("command runs");
    assert!(
        signed.status.success(),
        "privileged sign must succeed\nstderr:\n{}",
        String::from_utf8_lossy(&signed.stderr)
    );
    let signed_json = String::from_utf8_lossy(&signed.stdout).to_string();
    assert!(signed_json.contains("attestation"), "got: {signed_json}");

    // whip agent (unprivileged) verifies the signed envelope
    let signed_file = temp_path("gov-signed", "json");
    fs::write(&signed_file, &signed_json).expect("write signed");
    let verify = Command::new(bin)
        .args(["gov", "verify", signed_file.to_str().expect("utf-8 path")])
        .env_remove("WHIPPLESCRIPT_GOV_ADMIN")
        .output()
        .expect("command runs");
    assert!(
        verify.status.success(),
        "verify must succeed\nstderr:\n{}",
        String::from_utf8_lossy(&verify.stderr)
    );

    let _ = fs::remove_file(&config);
    let _ = fs::remove_file(&signed_file);
}

/// End-to-end trust root: a SIGNED envelope drives `whip check` enforcement, and a
/// tampered signed envelope is rejected (the whip agent refuses a tampered policy).
#[test]
fn ifc_check_enforces_and_rejects_tampered_signed_envelope() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let config = temp_path("gov-cfg2", "gov");
    fs::write(
        &config,
        "grant file_store ledger -> file:/srv/ledger.db readable by Operator\n\
         grant file_store outbox -> file:/srv/outbox public\n",
    )
    .expect("write config");

    // governance agent signs
    let signed = Command::new(bin)
        .args(["gov", "sign", config.to_str().expect("utf-8 path")])
        .env("WHIPPLESCRIPT_GOV_ADMIN", "1")
        .output()
        .expect("command runs");
    assert!(signed.status.success(), "sign should succeed");
    let signed_json = String::from_utf8_lossy(&signed.stdout).to_string();
    let signed_file = temp_path("gov-signed2", "json");
    fs::write(&signed_file, &signed_json).expect("write signed");

    let whip = temp_path("ifc-whip2", "whip");
    fs::write(&whip, IFC_BAD_WHIP).expect("write whip");

    // a verified signed envelope enforces: the bad flow is rejected
    let enforced = Command::new(bin)
        .args(["check", whip.to_str().expect("utf-8 path")])
        .env("WHIPPLESCRIPT_IFC_ENVELOPE", &signed_file)
        .output()
        .expect("command runs");
    assert!(!enforced.status.success(), "signed envelope should enforce");
    assert!(
        String::from_utf8_lossy(&enforced.stderr).contains("information-flow violation"),
        "stderr: {}",
        String::from_utf8_lossy(&enforced.stderr)
    );

    // tamper the signed envelope: flip ledger's reader to public without re-signing
    let tampered = signed_json.replace("\"reader\":\"Operator\"", "\"reader\":\"public\"");
    assert_ne!(tampered, signed_json, "tamper must change the content");
    fs::write(&signed_file, &tampered).expect("write tampered");
    let rejected = Command::new(bin)
        .args(["check", whip.to_str().expect("utf-8 path")])
        .env("WHIPPLESCRIPT_IFC_ENVELOPE", &signed_file)
        .output()
        .expect("command runs");
    assert!(
        !rejected.status.success(),
        "tampered envelope must be rejected"
    );
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("governance envelope rejected"),
        "stderr: {}",
        String::from_utf8_lossy(&rejected.stderr)
    );

    let _ = fs::remove_file(&config);
    let _ = fs::remove_file(&signed_file);
    let _ = fs::remove_file(&whip);
}

/// End-to-end escalation channel (DR-0028 D5): the whip side files an escalation
/// (unprivileged); only the governance agent (privileged) may review it.
#[test]
fn ifc_escalation_channel_whip_files_gov_reviews() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let log = temp_path("gov-escalations", "jsonl");
    let _ = fs::remove_file(&log);
    let log_arg = log.to_str().expect("utf-8 path");

    // whip side (unprivileged) files a request
    let filed = Command::new(bin)
        .args(["gov", "escalate", "need declassify ledger to Auditor"])
        .env("WHIPPLESCRIPT_GOV_ESCALATIONS", log_arg)
        .env_remove("WHIPPLESCRIPT_GOV_ADMIN")
        .output()
        .expect("command runs");
    assert!(
        filed.status.success(),
        "filing an escalation should succeed\nstderr:\n{}",
        String::from_utf8_lossy(&filed.stderr)
    );

    // whip side (unprivileged) cannot review
    let denied = Command::new(bin)
        .args(["gov", "escalations"])
        .env("WHIPPLESCRIPT_GOV_ESCALATIONS", log_arg)
        .env_remove("WHIPPLESCRIPT_GOV_ADMIN")
        .output()
        .expect("command runs");
    assert!(
        !denied.status.success(),
        "unprivileged review must be refused"
    );

    // governance agent (privileged) reviews the pending request
    let reviewed = Command::new(bin)
        .args(["gov", "escalations"])
        .env("WHIPPLESCRIPT_GOV_ESCALATIONS", log_arg)
        .env("WHIPPLESCRIPT_GOV_ADMIN", "1")
        .output()
        .expect("command runs");
    assert!(
        reviewed.status.success(),
        "privileged review should succeed"
    );
    assert!(
        String::from_utf8_lossy(&reviewed.stdout).contains("declassify ledger to Auditor"),
        "stdout: {}",
        String::from_utf8_lossy(&reviewed.stdout)
    );

    let _ = fs::remove_file(&log);
}

/// End-to-end governance agent loop (DR-0028 D5): privileged, it reads commands
/// from stdin, drafts a config, and signs; unprivileged, it refuses to start.
#[test]
fn ifc_governance_agent_loop_drafts_and_signs() {
    use std::io::Write;
    let bin = env!("CARGO_BIN_EXE_whip");
    let out = temp_path("gov-agent-out", "json");
    let _ = fs::remove_file(&out);
    let out_arg = out.to_str().expect("utf-8 path");
    let script =
        format!("grant file_store ledger -> file:/x readable by Operator\nsign {out_arg}\nquit\n");

    // privileged: the agent loop drafts and signs
    let mut child = Command::new(bin)
        .args(["gov", "agent"])
        .env("WHIPPLESCRIPT_GOV_ADMIN", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script");
    let output = child.wait_with_output().expect("wait");
    assert!(output.status.success(), "gov agent loop should succeed");
    let signed = fs::read_to_string(&out).expect("signed envelope written");
    assert!(signed.contains("attestation"), "got: {signed}");

    // unprivileged: the agent refuses to start
    let denied = Command::new(bin)
        .args(["gov", "agent"])
        .env_remove("WHIPPLESCRIPT_GOV_ADMIN")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("command runs");
    assert!(
        !denied.status.success(),
        "unprivileged gov agent must refuse"
    );

    let _ = fs::remove_file(&out);
}
