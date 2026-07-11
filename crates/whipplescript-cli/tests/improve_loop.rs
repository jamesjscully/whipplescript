//! End-to-end `whip improve` loop: pin a scenario from a real dev run,
//! campaign with the fixture proposer, dominance verdicts on real
//! regenerated evaluations, campaign record, and adoption — all through the
//! built binary with deterministic exec judges (no live provider).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "whip-improve-test-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

struct Env {
    dir: PathBuf,
    store: String,
    improve_store: String,
}

impl Env {
    fn new(label: &str) -> Self {
        let dir = temp_dir(label);
        let store = dir.join("store.sqlite").to_string_lossy().into_owned();
        let improve_store = dir.join("improve.sqlite").to_string_lossy().into_owned();
        Self {
            dir,
            store,
            improve_store,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_whip"));
        command
            .env("WHIPPLESCRIPT_EXEC_ALLOW", "python3 *")
            .env("WHIPPLESCRIPT_IMPROVE_STORE", &self.improve_store)
            .current_dir(&self.dir);
        command
    }

    fn run_json(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Value {
        let mut command = self.command();
        command.args(args);
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let output = command.output().expect("spawn whip");
        assert!(
            output.status.success(),
            "whip {args:?} failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let start = stdout.find('{').unwrap_or_else(|| {
            panic!("no JSON in output of whip {args:?}:\n{stdout}");
        });
        serde_json::from_str(&stdout[start..]).expect("parse JSON output")
    }

    fn run_expect_failure(&self, args: &[&str]) -> String {
        let mut command = self.command();
        command.args(args);
        let output = command.output().expect("spawn whip");
        assert!(
            !output.status.success(),
            "whip {args:?} unexpectedly succeeded"
        );
        String::from_utf8_lossy(&output.stderr).into_owned()
    }
}

const PRIORITY_JUDGE: &str = r#"
import json, sys
record = json.load(sys.stdin)
priority = None
for fact in record.get("facts", []):
    if fact.get("name") == "Assessment":
        priority = fact.get("value", {}).get("priority")
print(json.dumps({"ok": priority == "high"}))
"#;

const ECHO_JUDGE: &str = r#"
import json, sys
record = json.load(sys.stdin)
ticket_in = (record.get("input") or {}).get("ticket", {}).get("id")
echoed = None
for fact in record.get("facts", []):
    if fact.get("name") == "Assessment":
        echoed = fact.get("value", {}).get("ticket")
print(json.dumps({"passed": echoed == ticket_in}))
"#;

fn program(priority: &str, ticket_expr: &str, judge_dir: &std::path::Path) -> String {
    let priority_judge = judge_dir.join("judge_priority.py");
    let echo_judge = judge_dir.join("judge_echo.py");
    format!(
        r#"workflow Triage

input ticket Ticket

class Ticket {{
  id string
  title string
}}

class Assessment {{
  ticket string
  priority string
}}

gauge priority_correct {{
  judge via exec "python3 {priority_judge}"
  expect P(ok) at least 0.5
}}

gauge ticket_echoed {{
  judge via exec "python3 {echo_judge}"
}}

rule triage
  when Ticket as ticket
=> {{
  record Assessment {{
    ticket {ticket_expr}
    priority "{priority}"
  }}
}}
"#,
        priority_judge = priority_judge.display(),
        echo_judge = echo_judge.display(),
    )
}

fn write_judges(dir: &std::path::Path) {
    fs::write(dir.join("judge_priority.py"), PRIORITY_JUDGE).expect("write judge");
    fs::write(dir.join("judge_echo.py"), ECHO_JUDGE).expect("write judge");
}

fn dev_and_pin(env: &Env, program_path: &str) -> String {
    let dev = env.run_json(
        &[
            "--store",
            &env.store,
            "--input",
            r#"{"ticket":{"id":"T-1","title":"Fix login"}}"#,
            "--json",
            "dev",
            program_path,
            "--provider",
            "fixture",
        ],
        &[],
    );
    let instance = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();
    let pinned = env.run_json(
        &[
            "--store", &env.store, "--json", "pin", &instance, "--as", "case-1",
        ],
        &[],
    );
    assert_eq!(pinned["scenario"].as_str(), Some("case-1"));
    instance
}

#[test]
fn improve_campaign_proposes_dominant_candidate_and_adopts() {
    let env = Env::new("dominant");
    write_judges(&env.dir);
    let program_path = env.dir.join("triage.whip");
    let baseline_source = program("low", "ticket.id", &env.dir);
    fs::write(&program_path, &baseline_source).expect("write program");
    let program_str = program_path.to_string_lossy().into_owned();

    dev_and_pin(&env, &program_str);

    // Ambient scoring landed live evidence rows from the dev run.
    let gauges = env.run_json(&["--json", "gauges"], &[]);
    let ambient: Vec<&str> = gauges["gauges"]
        .as_array()
        .expect("gauges array")
        .iter()
        .filter_map(|gauge| gauge["gauge"].as_str())
        .collect();
    assert!(
        ambient.contains(&"priority_correct"),
        "ambient scoring records declared exec gauges: {ambient:?}"
    );

    // The fixture proposer offers the fixed program (priority high).
    let candidate_path = env.dir.join("candidate.whip");
    fs::write(&candidate_path, program("high", "ticket.id", &env.dir)).expect("write candidate");

    let report = env.run_json(
        &[
            "--json",
            "improve",
            "priority_correct",
            "--program",
            &program_str,
            "--proposer",
            "fixture",
        ],
        &[(
            "WHIPPLESCRIPT_IMPROVE_PROPOSALS",
            &candidate_path.to_string_lossy(),
        )],
    );
    assert_eq!(report["schema"].as_str(), Some("whipplescript.improve.v0"));
    assert_eq!(report["proposed"].as_bool(), Some(true));
    assert_eq!(
        report["unheld_out"].as_bool(),
        Some(true),
        "one pinned scenario is below the sealing floor — tagged, never blocked"
    );
    let cards = report["cards"].as_array().expect("cards");
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!(card["proposable"].as_bool(), Some(true));
    assert!(card["tags"]
        .as_array()
        .expect("tags")
        .iter()
        .any(|tag| tag.as_str() == Some("unheld-out")));
    let focus_line = card["gauges"]
        .as_array()
        .expect("gauge lines")
        .iter()
        .find(|line| line["gauge"].as_str() == Some("priority_correct"))
        .expect("focus gauge line");
    assert_eq!(focus_line["role"].as_str(), Some("ascend"));
    assert_eq!(focus_line["delta"].as_str(), Some("better"));
    assert_eq!(focus_line["bar_met"].as_bool(), Some(true));

    // The campaign record folded.
    let campaigns = env.run_json(&["--json", "campaigns"], &[]);
    let head = &campaigns["campaigns"].as_array().expect("campaigns")[0];
    assert_eq!(head["candidates"].as_i64(), Some(1));
    assert_eq!(head["proposed"].as_i64(), Some(1));
    let campaign_id = head["campaign"].as_str().expect("campaign id").to_owned();

    // Propose-don't-apply: the program on disk is untouched until adoption.
    assert_eq!(
        fs::read_to_string(&program_path).expect("read program"),
        baseline_source
    );
    let target = format!("{campaign_id}:K-1");
    let adopted = env.run_json(
        &["--json", "adopt", &target, "--program", &program_str],
        &[],
    );
    assert_eq!(adopted["candidate"].as_str(), Some("K-1"));
    let after = fs::read_to_string(&program_path).expect("read program");
    assert!(after.contains("priority \"high\""), "candidate adopted");

    // Adoption refuses when mainline moved under the campaign — the file
    // now holds the adopted candidate, which no longer matches the
    // campaign's baseline hash.
    let stderr = env.run_expect_failure(&["adopt", &target, "--program", &program_str]);
    assert!(
        stderr.contains("changed since campaign"),
        "stale adoption refused honestly: {stderr}"
    );
}

#[test]
fn improve_refuses_dominated_candidate_as_tradeoff() {
    let env = Env::new("dominated");
    write_judges(&env.dir);
    let program_path = env.dir.join("triage.whip");
    fs::write(&program_path, program("low", "ticket.id", &env.dir)).expect("write program");
    let program_str = program_path.to_string_lossy().into_owned();

    dev_and_pin(&env, &program_str);

    // The candidate improves the focus gauge but breaks the guarded gauge
    // (drops the ticket echo) — dominated, must not be proposed.
    let candidate_path = env.dir.join("candidate.whip");
    fs::write(&candidate_path, program("high", "\"wrong\"", &env.dir)).expect("write candidate");

    let report = env.run_json(
        &[
            "--json",
            "improve",
            "priority_correct",
            "--program",
            &program_str,
            "--proposer",
            "fixture",
        ],
        &[(
            "WHIPPLESCRIPT_IMPROVE_PROPOSALS",
            &candidate_path.to_string_lossy(),
        )],
    );
    assert_eq!(report["proposed"].as_bool(), Some(false));
    let cards = report["cards"].as_array().expect("cards");
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!(card["proposable"].as_bool(), Some(false));
    assert_eq!(
        card["tradeoff"].as_bool(),
        Some(true),
        "focus up + guard broken is a decision for the human, never an acceptance"
    );
    assert!(card["reasons"]
        .as_array()
        .expect("reasons")
        .iter()
        .any(|reason| reason.as_str().is_some_and(|r| r.contains("ticket_echoed"))));

    // The dominated candidate must not be adoptable as a proposal, but its
    // record exists for archaeology.
    let campaigns = env.run_json(&["--json", "campaigns"], &[]);
    let head = &campaigns["campaigns"].as_array().expect("campaigns")[0];
    assert_eq!(head["candidates"].as_i64(), Some(1));
    assert_eq!(head["proposed"].as_i64(), Some(0));
    let campaign_id = head["campaign"].as_str().expect("campaign id").to_owned();

    // Adoption is reserved for proposed candidates: the dominated candidate
    // is recorded but never adoptable (the acceptance model's invariant has
    // no side door).
    let target = format!("{campaign_id}:K-1");
    let stderr = env.run_expect_failure(&["adopt", &target, "--program", &program_str]);
    assert!(
        stderr.contains("was not proposed"),
        "dominated candidate adoption must be refused: {stderr}"
    );
}
