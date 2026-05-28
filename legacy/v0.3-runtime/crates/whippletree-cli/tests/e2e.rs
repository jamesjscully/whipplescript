use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;

struct Sandbox {
    root: TempDir,
    state: TempDir,
    bin: PathBuf,
}

impl Sandbox {
    fn new(config: &str) -> Self {
        let root = TempDir::new().expect("create workspace");
        let state = TempDir::new().expect("create state home");
        fs::create_dir_all(root.path().join(".whippletree")).expect("create config dir");
        fs::create_dir_all(root.path().join("scripts")).expect("create scripts dir");
        fs::write(root.path().join(".whippletree/project.whip"), config.trim())
            .expect("write config");

        Self {
            root,
            state,
            bin: PathBuf::from(env!("CARGO_BIN_EXE_whip")),
        }
    }

    fn root(&self) -> &Path {
        self.root.path()
    }

    fn write_script(&self, name: &str, contents: &str) {
        let path = self.root.path().join("scripts").join(name);
        fs::write(&path, contents.trim_start()).expect("write script");
        make_executable(&path);
    }

    fn write_file(&self, relative_path: &str, contents: &str) {
        let path = self.root.path().join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(path, contents).expect("write file");
    }

    fn whippletree<I, S>(&self, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Command::new(&self.bin)
            .arg("--workspace")
            .arg(self.root.path())
            .args(args)
            .env("XDG_STATE_HOME", self.state.path())
            .env("HOME", self.root.path())
            .current_dir(self.root.path())
            .output()
            .expect("run whippletree")
    }

    fn whippletree_with_stdin<I, S>(&self, args: I, stdin: &str) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut child = Command::new(&self.bin)
            .arg("--workspace")
            .arg(self.root.path())
            .args(args)
            .env("XDG_STATE_HOME", self.state.path())
            .env("HOME", self.root.path())
            .current_dir(self.root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn whippletree");
        let mut stdin_pipe = child.stdin.take().expect("open stdin");
        stdin_pipe.write_all(stdin.as_bytes()).expect("write stdin");
        drop(stdin_pipe);
        child.wait_with_output().expect("run whippletree")
    }

    fn ok_with_stdin<I, S>(&self, args: I, stdin: &str) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.whippletree_with_stdin(args, stdin);
        assert!(
            output.status.success(),
            "command failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn ok<I, S>(&self, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.whip(args);
        assert!(
            output.status.success(),
            "command failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn err<I, S>(&self, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.whip(args);
        assert!(
            !output.status.success(),
            "command unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn json<I, S>(&self, args: I) -> Value
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.ok(args);
        serde_json::from_slice(&output.stdout).expect("parse command json")
    }

    fn subscribe(&self, stream: &str) -> std::process::Child {
        Command::new(&self.bin)
            .arg("--workspace")
            .arg(self.root.path())
            .arg("subscribe")
            .arg(stream)
            .env("XDG_STATE_HOME", self.state.path())
            .env("HOME", self.root.path())
            .current_dir(self.root.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn subscribe")
    }

    fn wait_for_file(&self, relative_path: &str) {
        let path = self.root.path().join(relative_path);
        wait_until(|| path.is_file(), format!("{} to exist", path.display()));
    }

    fn read(&self, relative_path: &str) -> String {
        fs::read_to_string(self.root.path().join(relative_path)).expect("read file")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = Command::new(&self.bin)
            .arg("--workspace")
            .arg(self.root.path())
            .arg("down")
            .env("XDG_STATE_HOME", self.state.path())
            .env("HOME", self.root.path())
            .current_dir(self.root.path())
            .output();
    }
}

#[test]
fn event_service_lock_and_shutdown_flow_survives_real_cli_boundaries() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "on-smoke"
        on = "smoke.event"
        run = "sh scripts/on-event.sh"

        [[service]]
        name = "sleeper"
        run = "sh scripts/sleeper.sh"
        "#,
    );
    sandbox.write_script(
        "on-event.sh",
        &format!(
            r#"
        #!/bin/sh
        set -eu
        printf %s "$WHIPPLETREE_EVENT_TYPE" > event-type.txt
        printf %s "${{WHIPPLETREE_CORRELATION_ID:-}}" > event-correlation.txt
        printf %s "$WHIPPLETREE_EVENT_PAYLOAD_JSON" > event-payload.json
        printf %s "$WHIPPLETREE_PAYLOAD_JSON" > payload.json
        printf %s "$WHIPPLETREE_WORKSPACE" > workspace.txt
        printf %s "$WHIPPLETREE_WORKSPACE_ROOT" > workspace-root.txt
        printf %s "$WHIPPLETREE_CONFIG_DIR" > config-dir.txt
        printf %s "$WHIPPLETREE_STATE_DIR" > state-dir.txt
        printf %s "$WHIPPLETREE_RUN_DIR" > run-dir.txt
        "{}" emit child.event --source child-script --json '{{"child":true}}'
        "#,
            sandbox.bin.display()
        ),
    );
    sandbox.write_script(
        "sleeper.sh",
        r#"
        #!/bin/sh
        printf started > service-started.txt
        trap "exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );

    sandbox.ok(["--format", "json", "config", "check"]);
    assert_contains_stderr(sandbox.err(["tasks"]), "failed to connect to daemon");

    sandbox.ok(["up"]);
    sandbox.wait_for_file("service-started.txt");
    sandbox.ok(["up"]);
    assert_eq!(
        sandbox.json(["--format", "json", "tasks"])[0]["name"],
        "on-smoke"
    );
    assert_eq!(sandbox.json(["services", "--json"])[0]["name"], "sleeper");

    sandbox.ok([
        "emit",
        "smoke.event",
        "--source",
        "smoke-e2e",
        "--correlation",
        "corr-smoke",
        "--json",
        r#"{"answer":42,"correlationId":"corr-smoke"}"#,
    ]);
    sandbox.wait_for_file("event-type.txt");
    wait_until(
        || {
            let events = sandbox.json(["events", "--json"]);
            events.as_array().unwrap().iter().any(|event| {
                event["event_type"] == "child.event" && event["source"] == "child-script"
            })
        },
        "child event to be recorded",
    );
    assert_eq!(sandbox.read("event-type.txt"), "smoke.event");
    assert_eq!(sandbox.read("event-correlation.txt"), "corr-smoke");
    assert_eq!(
        sandbox.read("event-payload.json"),
        r#"{"answer":42,"correlationId":"corr-smoke"}"#
    );
    assert_eq!(
        sandbox.read("payload.json"),
        r#"{"answer":42,"correlationId":"corr-smoke"}"#
    );
    assert_eq!(
        sandbox.read("workspace.txt"),
        sandbox.root().display().to_string()
    );
    assert_eq!(
        sandbox.read("workspace-root.txt"),
        sandbox.root().display().to_string()
    );
    assert_eq!(
        sandbox.read("config-dir.txt"),
        sandbox.root().join(".whippletree").display().to_string()
    );
    assert!(!sandbox.read("state-dir.txt").is_empty());
    assert!(PathBuf::from(sandbox.read("run-dir.txt")).is_dir());

    let events = sandbox.json(["events", "--json"]);
    let smoke_event = events
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["event_type"] == "smoke.event" && event["source"] == "smoke-e2e")
        .expect("smoke event");
    let child_event = events
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["event_type"] == "child.event" && event["source"] == "child-script")
        .expect("child event");
    assert!(events
        .as_array()
        .unwrap()
        .iter()
        .any(|event| { event["event_type"] == "smoke.event" && event["source"] == "smoke-e2e" }));
    assert_eq!(child_event["parent_event_id"], smoke_event["id"]);
    assert_eq!(child_event["correlation_id"], "corr-smoke");
    assert!(child_event["source_run_id"]
        .as_str()
        .unwrap()
        .starts_with("run_"));
    let filtered_events = sandbox.json([
        "events",
        "--json",
        "--type",
        "smoke.event",
        "--source",
        "smoke-e2e",
        "--correlation",
        "corr-smoke",
        "--limit",
        "1",
    ]);
    assert_eq!(filtered_events.as_array().unwrap().len(), 1);
    assert_eq!(filtered_events[0]["event_type"], "smoke.event");
    let triggers = sandbox.json(["triggers", "--json"]);
    assert!(triggers
        .as_array()
        .unwrap()
        .iter()
        .any(|trigger| { trigger["task_name"] == "on-smoke" && trigger["outcome"] == "started" }));
    let filtered_triggers = sandbox.json([
        "triggers",
        "--json",
        "--task",
        "on-smoke",
        "--event-type",
        "smoke.event",
        "--outcome",
        "started",
        "--limit",
        "1",
    ]);
    assert_eq!(filtered_triggers.as_array().unwrap().len(), 1);
    assert_eq!(filtered_triggers[0]["task_name"], "on-smoke");

    assert_contains_stderr(
        sandbox.err(["lock", "acquire", "team-lock"]),
        "requires --ttl",
    );
    let acquired = sandbox.json([
        "--format",
        "json",
        "lock",
        "acquire",
        "team-lock",
        "--ttl",
        "1s",
        "--reason",
        "e2e",
    ]);
    let stale_token = acquired["token"].as_str().expect("lock token").to_string();
    assert_eq!(acquired["reason"], "e2e");
    assert_contains_stderr(
        sandbox.err(["lock", "acquire", "team-lock", "--ttl", "1s"]),
        "already held",
    );
    let locks = sandbox.json(["--format", "json", "lock", "status"]);
    assert!(locks.as_array().unwrap().iter().any(|lock| {
        lock["name"] == "team-lock"
            && lock["token"].as_str() == Some(stale_token.as_str())
            && lock["owner_id"].as_str().unwrap_or("").starts_with("pid:")
    }));
    thread::sleep(Duration::from_secs(2));
    let reacquired = sandbox.json([
        "--format",
        "json",
        "lock",
        "acquire",
        "team-lock",
        "--ttl",
        "1s",
    ]);
    let new_token = reacquired["token"]
        .as_str()
        .expect("new lock token")
        .to_string();
    assert_ne!(stale_token, new_token);
    assert_contains_stderr(
        sandbox.err([
            "lock",
            "release",
            "team-lock",
            "--token",
            stale_token.as_str(),
        ]),
        "different token",
    );
    let renewed = sandbox.json([
        "--format",
        "json",
        "lock",
        "renew",
        "team-lock",
        "--token",
        new_token.as_str(),
        "--ttl",
        "2s",
    ]);
    assert!(renewed["renewed_at_ms"].is_number());
    assert_contains_stderr(
        sandbox.err(["lock", "release", "team-lock"]),
        "requires --token",
    );
    sandbox.ok([
        "lock",
        "release",
        "team-lock",
        "--token",
        new_token.as_str(),
    ]);

    sandbox.ok(["down"]);
    assert_contains_stderr(
        sandbox.err(["emit", "smoke.event", "--json", "{}"]),
        "failed to connect to daemon",
    );
    assert_contains_stderr(
        sandbox.err(["lock", "status"]),
        "failed to connect to daemon",
    );
}

#[test]
fn emit_accepts_payload_file_and_stdin_for_agent_sources() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "on-agent"
        on = "agent.event"
        run = "true"
        "#,
    );

    sandbox.ok(["up"]);
    sandbox.write_file("payloads/file.json", r#"{"from":"file","n":1}"#);

    let from_file = sandbox.json([
        "--format",
        "json",
        "emit",
        "agent.event",
        "--source",
        "agent:file",
        "--payload-file",
        "payloads/file.json",
    ]);
    assert_eq!(from_file["emitted"], true);
    assert_eq!(from_file["event_type"], "agent.event");
    assert_eq!(from_file["payload"]["from"], "file");
    assert_eq!(from_file["payload_source"], "file:payloads/file.json");
    assert_eq!(from_file["source"], "agent:file");

    let output = sandbox.ok_with_stdin(
        [
            "--format",
            "json",
            "emit",
            "agent.event",
            "--source",
            "agent:stdin",
            "--stdin",
        ],
        r#"{"from":"stdin","n":2}"#,
    );
    let from_stdin: Value = serde_json::from_slice(&output.stdout).expect("parse stdin emit json");
    assert_eq!(from_stdin["payload"]["from"], "stdin");
    assert_eq!(from_stdin["payload_source"], "stdin");
    assert_eq!(from_stdin["source"], "agent:stdin");

    let events = sandbox.json(["events", "--json"]);
    assert!(events.as_array().unwrap().iter().any(|event| {
        event["event_type"] == "agent.event"
            && event["source"] == "agent:file"
            && event["payload"]["from"] == "file"
    }));
    assert!(events.as_array().unwrap().iter().any(|event| {
        event["event_type"] == "agent.event"
            && event["source"] == "agent:stdin"
            && event["payload"]["from"] == "stdin"
    }));

    sandbox.ok(["down"]);
}

#[test]
fn lock_fencing_tokens_are_enforced_from_the_cli() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "noop"
        run = "true"
        "#,
    );

    sandbox.ok(["up"]);

    assert_contains_stderr(
        sandbox.err(["lock", "acquire", "team-lock"]),
        "requires --ttl",
    );
    let acquired = sandbox.json([
        "--format",
        "json",
        "lock",
        "acquire",
        "team-lock",
        "--ttl",
        "100ms",
        "--reason",
        "fencing e2e",
    ]);
    let stale_token = acquired["token"].as_str().expect("lock token").to_string();
    assert_eq!(acquired["reason"], "fencing e2e");
    assert!(acquired["owner_id"]
        .as_str()
        .unwrap_or_default()
        .starts_with("pid:"));

    assert_contains_stderr(
        sandbox.err(["lock", "acquire", "team-lock", "--ttl", "1s"]),
        "already held",
    );
    let locks = sandbox.json(["--format", "json", "lock", "status"]);
    assert!(locks.as_array().unwrap().iter().any(|lock| {
        lock["name"] == "team-lock" && lock["token"].as_str() == Some(stale_token.as_str())
    }));

    thread::sleep(Duration::from_millis(150));
    let reacquired = sandbox.json([
        "--format",
        "json",
        "lock",
        "acquire",
        "team-lock",
        "--ttl",
        "1s",
    ]);
    let new_token = reacquired["token"].as_str().expect("new token").to_string();
    assert_ne!(stale_token, new_token);

    assert_contains_stderr(
        sandbox.err([
            "lock",
            "release",
            "team-lock",
            "--token",
            stale_token.as_str(),
        ]),
        "different token",
    );
    let renewed = sandbox.json([
        "--format",
        "json",
        "lock",
        "renew",
        "team-lock",
        "--token",
        new_token.as_str(),
        "--ttl",
        "2s",
    ]);
    assert!(renewed["renewed_at_ms"].is_number());
    assert_contains_stderr(
        sandbox.err(["lock", "release", "team-lock"]),
        "requires --token",
    );
    sandbox.ok([
        "lock",
        "release",
        "team-lock",
        "--token",
        new_token.as_str(),
    ]);
    assert!(sandbox
        .json(["--format", "json", "lock", "status"])
        .as_array()
        .unwrap()
        .is_empty());

    sandbox.ok(["down"]);
}

#[test]
fn lock_recovery_and_with_lock() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "noop"
        run = "true"
        "#,
    );

    sandbox.ok(["up"]);

    let expired = sandbox.json([
        "--format",
        "json",
        "lock",
        "acquire",
        "recoverable",
        "--ttl",
        "100ms",
        "--reason",
        "stale holder",
    ]);
    let expired_token = expired["token"]
        .as_str()
        .expect("expired token")
        .to_string();
    thread::sleep(Duration::from_millis(150));

    let expired_locks = sandbox.json(["--format", "json", "lock", "list", "--expired"]);
    assert!(expired_locks.as_array().unwrap().iter().any(|lock| {
        lock["name"] == "recoverable" && lock["token"].as_str() == Some(expired_token.as_str())
    }));

    let current = sandbox.json([
        "--format",
        "json",
        "lock",
        "acquire",
        "recoverable",
        "--ttl",
        "5s",
        "--reason",
        "new holder",
    ]);
    let current_token = current["token"]
        .as_str()
        .expect("current token")
        .to_string();
    assert_ne!(expired_token, current_token);
    assert_contains_stderr(
        sandbox.err([
            "lock",
            "release",
            "recoverable",
            "--token",
            expired_token.as_str(),
        ]),
        "different token",
    );

    let shown = sandbox.json(["--format", "json", "lock", "show", "recoverable"]);
    assert_eq!(shown["token"], current_token);
    assert_eq!(shown["reason"], "new holder");

    let forced = sandbox.json([
        "--format",
        "json",
        "lock",
        "force-release",
        "recoverable",
        "--reason",
        "operator recovery",
    ]);
    assert_eq!(forced["forced"], true);
    assert_eq!(forced["released"]["token"], current_token);
    assert!(sandbox
        .json(["--format", "json", "lock", "status"])
        .as_array()
        .unwrap()
        .is_empty());

    let audit_events = sandbox.json([
        "events",
        "--json",
        "--type",
        "lock.force_released",
        "--correlation",
        "recoverable",
    ]);
    assert!(audit_events.as_array().unwrap().iter().any(|event| {
        event["source"] == "lock"
            && event["payload"]["reason"] == "operator recovery"
            && event["payload"]["token"] == current_token
    }));

    sandbox.ok([
        "lock",
        "with",
        "wrapped",
        "--ttl",
        "5s",
        "--reason",
        "critical section",
        "--",
        "sh",
        "-c",
        "printf '%s:%s' \"$WHIPPLETREE_LOCK_NAME\" \"$WHIPPLETREE_LOCK_TOKEN\" > with-lock.txt",
    ]);
    let wrapped = sandbox.read("with-lock.txt");
    assert!(wrapped.starts_with("wrapped:lock_"));
    assert!(sandbox
        .json(["--format", "json", "lock", "status"])
        .as_array()
        .unwrap()
        .is_empty());

    let failed = sandbox.err([
        "lock",
        "with",
        "wrapped",
        "--ttl",
        "5s",
        "--reason",
        "failing section",
        "--",
        "sh",
        "-c",
        "exit 7",
    ]);
    assert_eq!(failed.status.code(), Some(7));
    assert!(sandbox
        .json(["--format", "json", "lock", "status"])
        .as_array()
        .unwrap()
        .is_empty());

    sandbox.ok(["down"]);
}

#[test]
fn object_cli_aliases() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "hello"
        run = "sh scripts/hello.sh"

        [[task]]
        name = "on-object"
        on = "object.event"
        run = "sh scripts/on-object.sh"

        [[service]]
        name = "worker"
        run = "sh scripts/worker.sh"
        "#,
    );
    sandbox.write_script(
        "hello.sh",
        r#"
        #!/bin/sh
        printf hello > hello.txt
        echo hello-output
        "#,
    );
    sandbox.write_script(
        "on-object.sh",
        r#"
        #!/bin/sh
        printf "$WHIPPLETREE_EVENT_TYPE" > event.txt
        "#,
    );
    sandbox.write_script(
        "worker.sh",
        r#"
        #!/bin/sh
        printf started > worker.txt
        trap "exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );

    sandbox.ok(["up"]);
    sandbox.wait_for_file("worker.txt");

    assert_eq!(
        sandbox.json(["--format", "json", "task", "list"])[0]["name"],
        "hello"
    );
    assert_eq!(
        sandbox.json(["--format", "json", "tasks"])[0]["name"],
        "hello"
    );
    assert_eq!(
        sandbox.json(["--format", "json", "task", "show", "hello"])["name"],
        "hello"
    );
    assert_eq!(
        sandbox.json(["--format", "json", "service", "list"])[0]["name"],
        "worker"
    );
    assert_eq!(
        sandbox.json(["--format", "json", "service", "show", "worker"])["name"],
        "worker"
    );
    assert_eq!(
        sandbox.json(["--format", "json", "services"])[0]["name"],
        "worker"
    );

    sandbox.ok(["task", "run", "hello"]);
    sandbox.wait_for_file("hello.txt");
    sandbox.ok(["run", "hello"]);

    let runs = sandbox.json(["--format", "json", "run", "list"]);
    let run_id = runs
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["name"] == "hello")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        sandbox.json(["--format", "json", "runs"])[0]["name"],
        runs[0]["name"]
    );
    assert_eq!(
        sandbox.json(["--format", "json", "run", "show", &run_id])["id"],
        run_id
    );
    sandbox.ok(["run", "logs", &run_id]);
    sandbox.ok(["log", "show", &run_id]);
    sandbox.ok(["log", "tail", &run_id, "--lines", "1"]);

    sandbox.ok(["event", "emit", "object.event", "--json", r#"{"ok":true}"#]);
    sandbox.wait_for_file("event.txt");
    sandbox.ok(["emit", "object.event", "--json", r#"{"alias":true}"#]);

    let events = sandbox.json(["--format", "json", "event", "list"]);
    let event_id = events[0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        sandbox.json(["--format", "json", "events"])[0]["event_type"],
        events[0]["event_type"]
    );
    assert_eq!(
        sandbox.json(["--format", "json", "event", "show", &event_id])["id"],
        event_id
    );

    wait_until(
        || {
            !sandbox
                .json(["--format", "json", "trigger", "list"])
                .as_array()
                .unwrap()
                .is_empty()
        },
        "trigger to be recorded",
    );
    let triggers = sandbox.json(["--format", "json", "triggers"]);
    let trigger_id = triggers[0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        sandbox.json(["--format", "json", "trigger", "show", &trigger_id])["id"],
        trigger_id
    );

    let overview = sandbox.json(["--format", "json", "overview"]);
    assert_eq!(overview["daemon_running"], true);
    assert!(overview["active_runs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|run| { run["name"] == "worker" && run["state"] == "running" }));
    assert!(overview["tasks"].as_array().unwrap().iter().any(|task| {
        task["name"] == "hello"
            && task["latest_run"]["name"] == "hello"
            && task["latest_run"]["state"] == "exited"
    }));
    assert!(overview["recent_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| { event["event_type"] == "object.event" }));
    assert!(overview["recent_triggers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|trigger| { trigger["task_name"] == "on-object" }));

    sandbox.ok(["down"]);
}

#[test]
fn manual_run_cancel_logs_and_runs_are_coherent_from_the_cli() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "long-task"
        run = "sh scripts/long-task.sh"
        "#,
    );
    sandbox.write_script(
        "long-task.sh",
        r#"
        #!/bin/sh
        set -eu
        echo "stdout-before-cancel"
        echo "stdout-tail-line"
        echo "stderr-before-cancel" >&2
        echo "stderr-tail-line" >&2
        printf %s "$WHIPPLETREE_RUN_ID" > active-run-id.txt
        trap "echo cancelled-cleanly; exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );

    sandbox.ok(["up"]);
    let started = sandbox.json(["--format", "json", "run", "long-task"]);
    let run_id = started["run_id"].as_str().expect("run id").to_string();
    sandbox.wait_for_file("active-run-id.txt");
    assert_eq!(sandbox.read("active-run-id.txt"), run_id);
    wait_until(
        || {
            let logs = sandbox.json(["--format", "json", "logs", &run_id]);
            logs["stderr"]
                .as_str()
                .map(|stderr| stderr.contains("stderr-tail-line"))
                .unwrap_or(false)
        },
        "stderr tail line to flush",
    );

    let ps = sandbox.json(["ps", "--json"]);
    assert!(ps["runs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|run| run["id"] == run_id && run["state"] == "running"));

    sandbox.ok(["cancel", &run_id]);
    wait_until(
        || {
            let runs = sandbox.json(["runs", "--json"]);
            runs.as_array().unwrap().iter().any(|run| {
                run["id"] == run_id
                    && matches!(
                        run["state"].as_str(),
                        Some("succeeded" | "cancelled" | "failed")
                    )
            })
        },
        "cancelled run to be persisted",
    );
    let filtered_runs = sandbox.json([
        "runs",
        "--json",
        "--name",
        "long-task",
        "--origin",
        "task",
        "--limit",
        "1",
    ]);
    assert_eq!(filtered_runs.as_array().unwrap().len(), 1);
    assert_eq!(filtered_runs[0]["id"], run_id);

    let logs = sandbox.json(["--format", "json", "logs", &run_id]);
    assert!(logs["stdout"]
        .as_str()
        .unwrap()
        .contains("stdout-before-cancel"));
    assert!(logs["stderr"]
        .as_str()
        .unwrap()
        .contains("stderr-before-cancel"));
    let tailed_logs = sandbox.json(["--format", "json", "logs", "--tail", "2", &run_id]);
    assert!(tailed_logs["stdout"]
        .as_str()
        .unwrap()
        .contains("cancelled-cleanly"));
    assert!(!tailed_logs["stdout"]
        .as_str()
        .unwrap()
        .contains("stdout-before-cancel"));
    assert_contains_stderr(
        sandbox.err(["--format", "json", "logs", "--follow", &run_id]),
        "cannot be combined with JSON output",
    );
    sandbox.ok(["down"]);
}

#[test]
fn logs_cli_shows_metadata_stream_stats_and_tails_output() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "loggy"
        run = "sh scripts/loggy.sh"
        "#,
    );
    sandbox.write_script(
        "loggy.sh",
        r#"
        #!/bin/sh
        set -eu
        printf 'stdout-1\nstdout-2\nstdout-3\n'
        printf 'stderr-1\nstderr-2\nstderr-3\n' >&2
        "#,
    );

    sandbox.ok(["up"]);
    let started = sandbox.json(["--format", "json", "run", "loggy"]);
    let run_id = started["run_id"].as_str().expect("run id").to_string();
    wait_until(
        || {
            let runs = sandbox.json(["runs", "--json"]);
            runs.as_array().unwrap().iter().any(|run| {
                run["id"] == run_id && matches!(run["state"].as_str(), Some("exited" | "failed"))
            })
        },
        "loggy run to finish",
    );

    let text_output = sandbox.ok(["logs", "--tail", "2", &run_id]);
    let text = String::from_utf8_lossy(&text_output.stdout);
    assert!(text.contains(&format!("run {run_id}")));
    assert!(text.contains("name: loggy"));
    assert!(text.contains("origin: task  state: exited"));
    assert!(text.contains("tail: last 2 lines per stream"));
    assert!(text.contains("stdout "));
    assert!(text.contains("stderr "));
    assert!(text.contains("stdout-2"));
    assert!(text.contains("stdout-3"));
    assert!(!text.contains("stdout-1"));
    assert!(text.contains("stderr-2"));
    assert!(text.contains("stderr-3"));
    assert!(!text.contains("stderr-1"));
    assert!(text.contains("truncated"));

    let logs = sandbox.json(["--format", "json", "logs", "--tail", "2", &run_id]);
    assert_eq!(logs["run"]["name"], "loggy");
    assert_eq!(logs["run"]["state"], "exited");
    assert!(logs["run_directory"].as_str().unwrap().contains(&run_id));
    assert_eq!(logs["stdout_lines"], 3);
    assert_eq!(logs["stderr_lines"], 3);
    assert!(logs["stdout_bytes"].as_u64().unwrap() > 0);
    assert!(logs["stderr_bytes"].as_u64().unwrap() > 0);
    assert_eq!(logs["stdout_truncated"], true);
    assert_eq!(logs["stderr_truncated"], true);
    assert_eq!(logs["stdout_missing"], false);
    assert_eq!(logs["stderr_missing"], false);
    assert!(!logs["stdout"].as_str().unwrap().contains("stdout-1"));
    assert!(logs["stdout"].as_str().unwrap().contains("stdout-3"));
    sandbox.ok(["down"]);
}

#[test]
fn daemon_restart_marks_previously_active_runs_as_failed_and_inspectable() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "long-task"
        run = "sh scripts/long-task.sh"
        "#,
    );
    sandbox.write_script(
        "long-task.sh",
        r#"
        #!/bin/sh
        set -eu
        echo "stdout-before-daemon-death"
        echo "stderr-before-daemon-death" >&2
        printf %s "$WHIPPLETREE_RUN_ID" > active-run-id.txt
        while [ ! -f stop-long-task ]; do sleep 1; done
        "#,
    );

    sandbox.ok(["up"]);
    let started = sandbox.json(["--format", "json", "run", "long-task"]);
    let run_id = started["run_id"].as_str().expect("run id").to_string();
    sandbox.wait_for_file("active-run-id.txt");

    let status = sandbox.json(["status", "--json"]);
    let pid_path = status["pid_path"].as_str().expect("pid path");
    let pid = fs::read_to_string(pid_path)
        .expect("read daemon pid")
        .trim()
        .parse::<libc::pid_t>()
        .expect("parse daemon pid");
    assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
    wait_until(
        || !sandbox.whip(["status"]).status.success(),
        "daemon to be unreachable after SIGKILL",
    );

    sandbox.ok(["up"]);
    let ps = sandbox.json(["ps", "--json"]);
    assert!(ps["runs"].as_array().unwrap().is_empty());

    let runs = sandbox.json(["runs", "--json"]);
    assert!(runs.as_array().unwrap().iter().any(|run| {
        run["id"] == run_id
            && run["name"] == "long-task"
            && run["state"] == "failed"
            && run["exit_code"].is_null()
            && run["signal"].is_null()
    }));

    let logs = sandbox.json(["--format", "json", "logs", &run_id]);
    assert!(logs["stdout"]
        .as_str()
        .unwrap()
        .contains("stdout-before-daemon-death"));
    assert!(logs["stderr"]
        .as_str()
        .unwrap()
        .contains("marked failed during daemon startup"));
    assert_eq!(logs["run"]["state"], "failed");
    assert_eq!(logs["run"]["exit_code"], Value::Null);
    assert_eq!(logs["run"]["signal"], Value::Null);

    let run_directory = logs["run_directory"].as_str().expect("run directory");
    let meta: Value = serde_json::from_str(
        &fs::read_to_string(Path::new(run_directory).join("meta.json")).expect("read run meta"),
    )
    .expect("parse run meta");
    assert_eq!(meta["state"], "failed");
    assert_eq!(meta["end_time"], logs["run"]["end_time"]);
    assert_eq!(meta["exit_code"], Value::Null);
    assert_eq!(meta["signal"], Value::Null);

    sandbox.write_file("stop-long-task", "");
    sandbox.ok(["down"]);
}

#[test]
fn invalid_reload_preserves_running_daemon_and_valid_config_can_reload() {
    let good_config = r#"
        [[task]]
        name = "on-smoke"
        on = "smoke.event"
        run = "sh scripts/on-event.sh"

        [[service]]
        name = "worker"
        run = "sh scripts/worker.sh"
    "#;
    let sandbox = Sandbox::new(good_config);
    sandbox.write_script(
        "on-event.sh",
        r#"
        #!/bin/sh
        printf event > event-after-invalid-reload.txt
        "#,
    );
    sandbox.write_script(
        "worker.sh",
        r#"
        #!/bin/sh
        printf started > worker-started.txt
        trap "exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );

    sandbox.ok(["up"]);
    sandbox.wait_for_file("worker-started.txt");
    sandbox.write_file(
        ".whippletree/project.whip",
        "[[service]]\nname = \"broken\"\nrun =\n",
    );
    assert_contains_stderr(sandbox.err(["up"]), "invalid TOML config");
    sandbox.ok(["status"]);
    sandbox.ok(["emit", "smoke.event", "--json", "{}"]);
    sandbox.wait_for_file("event-after-invalid-reload.txt");

    sandbox.write_file(".whippletree/project.whip", good_config);
    sandbox.ok(["up"]);
    sandbox.ok(["down"]);
}

#[test]
fn reload_removing_in_flight_task_and_service_stays_observable() {
    let initial_config = r#"
        [[task]]
        name = "old-task"
        run = "sh scripts/old-task.sh"

        [[service]]
        name = "old-service"
        run = "sh scripts/old-service.sh"
    "#;
    let sandbox = Sandbox::new(initial_config);
    sandbox.write_script(
        "old-task.sh",
        r#"
        #!/bin/sh
        printf started > old-task-started.txt
        sleep 1
        printf done > old-task-done.txt
        "#,
    );
    sandbox.write_script(
        "old-service.sh",
        r#"
        #!/bin/sh
        printf started > old-service-started.txt
        trap "printf stopped > old-service-stopped.txt; exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );

    sandbox.ok(["up"]);
    sandbox.wait_for_file("old-service-started.txt");
    let started = sandbox.json(["--format", "json", "run", "old-task"]);
    let task_run_id = started["run_id"].as_str().expect("run id").to_string();
    sandbox.wait_for_file("old-task-started.txt");

    sandbox.write_file(
        ".whippletree/project.whip",
        r#"
        [[task]]
        name = "new-task"
        run = "true"
        "#,
    );
    sandbox.ok(["up"]);

    let tasks = sandbox.json(["--format", "json", "tasks"]);
    assert!(tasks
        .as_array()
        .unwrap()
        .iter()
        .any(|task| task["name"] == "new-task"));
    assert!(!tasks
        .as_array()
        .unwrap()
        .iter()
        .any(|task| task["name"] == "old-task"));
    assert!(sandbox
        .json(["services", "--json"])
        .as_array()
        .unwrap()
        .is_empty());

    sandbox.wait_for_file("old-service-stopped.txt");
    sandbox.wait_for_file("old-task-done.txt");
    sandbox.ok(["status"]);
    let runs = sandbox.json(["runs", "--json"]);
    assert!(runs.as_array().unwrap().iter().any(|run| {
        run["id"] == task_run_id && run["name"] == "old-task" && run["state"] == "exited"
    }));

    sandbox.ok(["down"]);
}

#[test]
fn reload_changed_service_stops_old_run_and_starts_new_command() {
    let initial_config = r#"
        [[service]]
        name = "worker"
        run = "sh scripts/worker-v1.sh"
    "#;
    let sandbox = Sandbox::new(initial_config);
    sandbox.write_script(
        "worker-v1.sh",
        r#"
        #!/bin/sh
        printf v1 > worker-v1-started.txt
        trap "printf stopped > worker-v1-stopped.txt; exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );
    sandbox.write_script(
        "worker-v2.sh",
        r#"
        #!/bin/sh
        printf v2 > worker-v2-started.txt
        trap "exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );

    sandbox.ok(["up"]);
    sandbox.wait_for_file("worker-v1-started.txt");
    sandbox.write_file(
        ".whippletree/project.whip",
        r#"
        [[service]]
        name = "worker"
        run = "sh scripts/worker-v2.sh"
        "#,
    );
    sandbox.ok(["up"]);

    sandbox.wait_for_file("worker-v1-stopped.txt");
    sandbox.wait_for_file("worker-v2-started.txt");
    let services = sandbox.json(["services", "--json"]);
    assert_eq!(services[0]["name"], "worker");
    assert_eq!(services[0]["state"], "running");

    sandbox.ok(["down"]);
}

#[test]
fn restart_cancels_in_flight_runs_and_comes_back_on_latest_config() {
    let initial_config = r#"
        [[task]]
        name = "long-task"
        run = "sh scripts/long-task.sh"

        [[service]]
        name = "worker"
        run = "sh scripts/worker-v1.sh"
    "#;
    let sandbox = Sandbox::new(initial_config);
    sandbox.write_script(
        "long-task.sh",
        r#"
        #!/bin/sh
        printf started > long-task-started.txt
        trap "printf stopped > long-task-stopped.txt; exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );
    sandbox.write_script(
        "worker-v1.sh",
        r#"
        #!/bin/sh
        printf v1 > restart-worker-v1-started.txt
        trap "printf stopped > restart-worker-v1-stopped.txt; exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );
    sandbox.write_script(
        "worker-v2.sh",
        r#"
        #!/bin/sh
        printf v2 > restart-worker-v2-started.txt
        trap "exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );

    sandbox.ok(["up"]);
    sandbox.wait_for_file("restart-worker-v1-started.txt");
    sandbox.ok(["run", "long-task"]);
    sandbox.wait_for_file("long-task-started.txt");

    sandbox.write_file(
        ".whippletree/project.whip",
        r#"
        [[task]]
        name = "new-task"
        run = "true"

        [[service]]
        name = "worker"
        run = "sh scripts/worker-v2.sh"
        "#,
    );
    sandbox.ok(["restart"]);

    sandbox.wait_for_file("long-task-stopped.txt");
    sandbox.wait_for_file("restart-worker-v1-stopped.txt");
    sandbox.wait_for_file("restart-worker-v2-started.txt");
    let status = sandbox.json(["--format", "json", "status"]);
    assert_eq!(status["tasks"], 1);
    assert_eq!(status["services"], 1);
    assert!(sandbox
        .json(["--format", "json", "tasks"])
        .as_array()
        .unwrap()
        .iter()
        .any(|task| task["name"] == "new-task"));

    sandbox.ok(["down"]);
}

#[test]
fn file_watch_trigger_runs_from_real_daemon_polling() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "watcher"
        watch = ["watched.txt"]
        settle = "50ms"
        run = "sh scripts/on-watch.sh"
        "#,
    );
    sandbox.write_file("watched.txt", "initial\n");
    sandbox.write_script(
        "on-watch.sh",
        r#"
        #!/bin/sh
        count=0
        if [ -f watch-count.txt ]; then
          count=$(cat watch-count.txt)
        fi
        count=$((count + 1))
        printf %s "$count" > watch-count.txt
        "#,
    );

    sandbox.ok(["up"]);
    thread::sleep(Duration::from_millis(200));
    sandbox.write_file("watched.txt", "changed\n");
    sandbox.wait_for_file("watch-count.txt");
    let events = sandbox.json(["events", "--json"]);
    assert!(events
        .as_array()
        .unwrap()
        .iter()
        .any(|event| { event["event_type"] == "file.changed" && event["routing"] == "watch" }));
    let triggers = sandbox.json(["triggers", "--json"]);
    assert!(triggers
        .as_array()
        .unwrap()
        .iter()
        .any(|trigger| { trigger["task_name"] == "watcher" && trigger["outcome"] == "started" }));
    sandbox.ok(["down"]);
}

#[test]
fn trigger_delivery_semantics_are_inspectable_from_the_cli() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "rejector"
        on = "delivery.reject"
        run = "sh scripts/record-trigger.sh reject.log 0.5"

        [task.admission]
        when_busy = "reject"

        [[task]]
        name = "queue-one"
        on = "delivery.queue"
        run = "sh scripts/record-trigger.sh queue.log 0.4"

        [task.admission]
        when_busy = "queue_one"

        [[task]]
        name = "restarter"
        on = "delivery.restart"
        run = "sh scripts/record-trigger.sh restart.log 1"

        [task.admission]
        when_busy = "restart"
        "#,
    );
    sandbox.write_script(
        "record-trigger.sh",
        r#"
        #!/bin/sh
        set -eu
        printf '%s %s\n' "$WHIPPLETREE_EVENT_ID" "$WHIPPLETREE_EVENT_PAYLOAD_JSON" >> "$1"
        sleep "$2"
        "#,
    );

    sandbox.ok(["up"]);

    sandbox.ok(["emit", "delivery.unmatched", "--json", r#"{"n":0}"#]);
    let events = sandbox.json(["events", "--json"]);
    assert_eq!(count_events(&events, "delivery.unmatched"), 1);
    let triggers = sandbox.json(["triggers", "--json"]);
    assert_eq!(count_triggers(&triggers, "delivery.unmatched", None), 0);

    sandbox.ok(["emit", "delivery.reject", "--json", r#"{"n":1}"#]);
    wait_until(
        || task_active(&sandbox, "rejector"),
        "rejector to have an active run",
    );
    sandbox.ok(["emit", "delivery.reject", "--json", r#"{"n":2}"#]);

    sandbox.ok(["emit", "delivery.queue", "--json", r#"{"n":1}"#]);
    wait_until(
        || task_active(&sandbox, "queue-one"),
        "queue-one to have an active run",
    );
    sandbox.ok(["emit", "delivery.queue", "--json", r#"{"n":2}"#]);
    sandbox.ok(["emit", "delivery.queue", "--json", r#"{"n":3}"#]);

    sandbox.ok(["emit", "delivery.restart", "--json", r#"{"n":1}"#]);
    wait_until(
        || task_active(&sandbox, "restarter"),
        "restarter to have an active run",
    );
    sandbox.ok(["emit", "delivery.restart", "--json", r#"{"n":2}"#]);
    sandbox.ok(["emit", "delivery.restart", "--json", r#"{"n":3}"#]);

    wait_until(
        || file_line_count(&sandbox, "queue.log") >= 2,
        "queue-one to start the first run and one queued run",
    );
    wait_until(
        || file_line_count(&sandbox, "restart.log") >= 2,
        "restart admission to start a replacement run",
    );

    let events = sandbox.json(["events", "--json"]);
    assert_eq!(count_events(&events, "delivery.reject"), 2);
    assert_eq!(count_events(&events, "delivery.queue"), 3);
    assert_eq!(count_events(&events, "delivery.restart"), 3);

    let triggers = sandbox.json(["triggers", "--json"]);
    assert!(has_trigger(&triggers, "delivery.reject", "started"));
    assert!(has_trigger(&triggers, "delivery.reject", "rejected"));
    assert!(has_trigger(&triggers, "delivery.queue", "queued"));
    assert!(has_trigger(&triggers, "delivery.queue", "coalesced"));
    assert!(has_trigger(&triggers, "delivery.restart", "queued"));
    assert!(has_trigger(&triggers, "delivery.restart", "superseded"));
    assert!(has_trigger(&triggers, "delivery.restart", "started"));

    assert_eq!(file_line_count(&sandbox, "queue.log"), 2);
    assert!(sandbox.read("queue.log").contains(r#"{"n":2}"#));
    assert!(!sandbox.read("queue.log").contains(r#"{"n":3}"#));
    assert!(sandbox.read("restart.log").contains(r#"{"n":3}"#));

    sandbox.ok(["down"]);
}

#[test]
fn dynamic_service_lifecycle() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "placeholder"
        run = "true"
        "#,
    );
    let original_config = sandbox.read(".whippletree/project.whip");
    sandbox.write_script(
        "dynamic-service.sh",
        r#"
        #!/bin/sh
        set -eu
        printf '%s\n' "${MARKER:-missing}:${WHIPPLETREE_NAME}:${WHIPPLETREE_KIND}" >> dynamic-started.log
        trap "printf '%s\n' \"${MARKER:-missing}:${WHIPPLETREE_NAME}\" >> dynamic-stopped.log; exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );

    sandbox.ok(["up"]);
    sandbox.ok([
        "service",
        "add",
        "dyn-source",
        "--correlation",
        "dyn-correlation",
        "--env",
        "MARKER=dynamic",
        "--restart",
        "never",
        "--reason",
        "e2e dynamic service",
        "--",
        "sh",
        "scripts/dynamic-service.sh",
    ]);
    sandbox.ok([
        "wait",
        "service",
        "dyn-source",
        "--state",
        "running",
        "--timeout",
        "5s",
    ]);
    sandbox.wait_for_file("dynamic-started.log");

    let dynamic_services = sandbox.json(["--format", "json", "service", "list", "--dynamic"]);
    assert_eq!(dynamic_services.as_array().unwrap().len(), 1);
    assert_eq!(dynamic_services[0]["name"], "dyn-source");
    assert_eq!(dynamic_services[0]["dynamic"], true);
    assert_eq!(dynamic_services[0]["correlation_id"], "dyn-correlation");
    assert_eq!(dynamic_services[0]["reason"], "e2e dynamic service");
    assert_eq!(dynamic_services[0]["env"][0][0], "MARKER");
    assert_eq!(dynamic_services[0]["env"][0][1], "dynamic");

    let shown = sandbox.json(["--format", "json", "service", "show", "dyn-source"]);
    assert_eq!(shown["dynamic"], true);
    assert_eq!(shown["run"], "sh scripts/dynamic-service.sh");
    assert_eq!(sandbox.read(".whippletree/project.whip"), original_config);

    let first_run = service_active_run_id(&sandbox, "dyn-source").expect("active run id");
    sandbox.ok(["service", "restart", "dyn-source"]);
    wait_until(
        || {
            service_active_run_id(&sandbox, "dyn-source")
                .map(|run_id| run_id != first_run)
                .unwrap_or(false)
        },
        "dynamic service restart to replace active run",
    );

    sandbox.ok(["service", "remove", "dyn-source"]);
    wait_until(
        || {
            sandbox
                .json(["--format", "json", "service", "list", "--dynamic"])
                .as_array()
                .unwrap()
                .is_empty()
        },
        "dynamic service to be removed",
    );
    sandbox.wait_for_file("dynamic-stopped.log");
    assert_eq!(sandbox.read(".whippletree/project.whip"), original_config);

    sandbox.ok([
        "service",
        "add",
        "shutdown-only",
        "--",
        "sh",
        "scripts/dynamic-service.sh",
    ]);
    sandbox.ok([
        "wait",
        "service",
        "shutdown-only",
        "--state",
        "running",
        "--timeout",
        "5s",
    ]);
    sandbox.ok(["down"]);
    sandbox.ok(["up"]);
    assert!(sandbox
        .json(["--format", "json", "service", "list", "--dynamic"])
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(sandbox.read(".whippletree/project.whip"), original_config);
    sandbox.ok(["down"]);
}

#[test]
fn dynamic_task_event_and_watch_lifecycle() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "placeholder"
        run = "true"
        "#,
    );
    let original_config = sandbox.read(".whippletree/project.whip");
    sandbox.write_script(
        "dynamic-event.sh",
        r#"
        #!/bin/sh
        set -eu
        printf '%s|%s|%s|%s\n' "$WHIPPLETREE_NAME" "$WHIPPLETREE_KIND" "${MARKER:-missing}" "$WHIPPLETREE_EVENT_PAYLOAD_JSON" >> dynamic-event.log
        "#,
    );
    sandbox.write_script(
        "dynamic-watch.sh",
        r#"
        #!/bin/sh
        set -eu
        printf '%s|%s\n' "$WHIPPLETREE_NAME" "$WHIPPLETREE_EVENT_PAYLOAD_JSON" >> dynamic-watch.log
        "#,
    );
    sandbox.write_script(
        "dynamic-schedule.sh",
        r#"
        #!/bin/sh
        set -eu
        printf '%s|%s\n' "$WHIPPLETREE_NAME" "$WHIPPLETREE_EVENT_TYPE" >> dynamic-schedule.log
        "#,
    );

    sandbox.ok(["up"]);
    sandbox.ok([
        "task",
        "add",
        "dyn-event",
        "--on",
        "dynamic.event",
        "--correlation",
        "dyn-task-corr",
        "--env",
        "MARKER=event-env",
        "--",
        "sh",
        "scripts/dynamic-event.sh",
    ]);
    sandbox.ok([
        "task",
        "add",
        "dyn-watch",
        "--watch",
        "watched/dynamic.txt",
        "--settle",
        "50ms",
        "--",
        "sh",
        "scripts/dynamic-watch.sh",
    ]);
    sandbox.ok([
        "task",
        "add",
        "dyn-schedule",
        "--schedule",
        "*/1 * * * * *",
        "--",
        "sh",
        "scripts/dynamic-schedule.sh",
    ]);

    let dynamic_tasks = sandbox.json(["--format", "json", "task", "list", "--dynamic"]);
    assert_eq!(dynamic_tasks.as_array().unwrap().len(), 3);
    assert!(dynamic_tasks
        .as_array()
        .unwrap()
        .iter()
        .all(|task| task["dynamic"] == true));
    let shown = sandbox.json(["--format", "json", "task", "show", "dyn-event"]);
    assert_eq!(shown["dynamic"], true);
    assert_eq!(shown["run"], "sh scripts/dynamic-event.sh");
    assert_eq!(shown["on"], "dynamic.event");
    assert_eq!(shown["correlation_id"], "dyn-task-corr");
    assert_eq!(shown["env"][0][0], "MARKER");
    assert_eq!(shown["env"][0][1], "event-env");
    assert_eq!(sandbox.read(".whippletree/project.whip"), original_config);

    sandbox.ok(["emit", "dynamic.event", "--json", r#"{"kind":"event"}"#]);
    sandbox.wait_for_file("dynamic-event.log");
    assert!(sandbox
        .read("dynamic-event.log")
        .contains(r#"dyn-event|task|event-env|{"kind":"event"}"#));
    sandbox.ok([
        "wait",
        "trigger",
        "--task",
        "dyn-event",
        "--outcome",
        "started",
        "--timeout",
        "5s",
    ]);

    sandbox.write_file("watched/dynamic.txt", "first");
    sandbox.wait_for_file("dynamic-watch.log");
    let watch_log = sandbox.read("dynamic-watch.log");
    assert!(watch_log.contains("dyn-watch|"));
    assert!(watch_log.contains("watched/dynamic.txt"));

    sandbox.wait_for_file("dynamic-schedule.log");
    assert!(sandbox
        .read("dynamic-schedule.log")
        .contains("dyn-schedule|timer.fired"));

    let event_trigger_count = count_triggers_for_task(&sandbox, "dyn-event", "started");
    let watch_trigger_count = count_triggers_for_task(&sandbox, "dyn-watch", "started");
    let schedule_lines = file_line_count(&sandbox, "dynamic-schedule.log");
    sandbox.ok(["task", "remove", "dyn-event"]);
    sandbox.ok(["task", "remove", "dyn-watch"]);
    sandbox.ok(["task", "remove", "dyn-schedule"]);
    wait_until(
        || {
            sandbox
                .json(["--format", "json", "task", "list", "--dynamic"])
                .as_array()
                .unwrap()
                .is_empty()
        },
        "dynamic tasks to be removed",
    );

    sandbox.ok([
        "emit",
        "dynamic.event",
        "--json",
        r#"{"kind":"after-remove"}"#,
    ]);
    sandbox.write_file("watched/dynamic.txt", "second");
    thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        count_triggers_for_task(&sandbox, "dyn-event", "started"),
        event_trigger_count
    );
    assert_eq!(
        count_triggers_for_task(&sandbox, "dyn-watch", "started"),
        watch_trigger_count
    );
    assert_eq!(
        file_line_count(&sandbox, "dynamic-schedule.log"),
        schedule_lines
    );
    assert_eq!(sandbox.read(".whippletree/project.whip"), original_config);

    sandbox.ok([
        "task",
        "add",
        "shutdown-only",
        "--on",
        "shutdown.dynamic",
        "--",
        "true",
    ]);
    sandbox.ok(["down"]);
    sandbox.ok(["up"]);
    assert!(sandbox
        .json(["--format", "json", "task", "list", "--dynamic"])
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(sandbox.read(".whippletree/project.whip"), original_config);
    sandbox.ok(["down"]);
}

#[test]
fn wait_and_subscribe_agent_flow() {
    let sandbox = Sandbox::new(
        r#"
        [[task]]
        name = "observer"
        on = "query.request"
        run = "sh scripts/observer.sh"
        "#,
    );
    sandbox.write_script(
        "observer.sh",
        r#"
        #!/bin/sh
        set -eu
        printf '%s' "$WHIPPLETREE_RUN_ID" > active-run-id.txt
        printf '%s' "${WHIPPLETREE_CORRELATION_ID:-}" > correlation.txt
        "#,
    );

    sandbox.ok(["up"]);

    let mut event_subscriber = sandbox.subscribe("events");
    let mut run_subscriber = sandbox.subscribe("runs");

    sandbox.ok([
        "emit",
        "query.request",
        "--source",
        "wait-e2e",
        "--correlation",
        "corr-wait",
        "--json",
        r#"{"ok":true}"#,
    ]);

    let waited_event = sandbox.json([
        "--format",
        "json",
        "wait",
        "event",
        "query.request",
        "--correlation",
        "corr-wait",
        "--timeout",
        "5s",
    ]);
    assert_eq!(waited_event["type"], "query.request");
    assert_eq!(waited_event["correlation_id"], "corr-wait");

    let subscribed_event = read_ndjson_until(&mut event_subscriber, |value| {
        value["type"] == "query.request" && value["correlation_id"] == "corr-wait"
    });
    assert_eq!(subscribed_event["source"], "wait-e2e");

    let waited_trigger = sandbox.json([
        "--format",
        "json",
        "wait",
        "trigger",
        "--task",
        "observer",
        "--outcome",
        "started",
        "--correlation",
        "corr-wait",
        "--timeout",
        "5s",
    ]);
    assert_eq!(waited_trigger["task_name"], "observer");
    assert_eq!(waited_trigger["outcome"], "started");

    sandbox.wait_for_file("active-run-id.txt");
    let run_id = sandbox.read("active-run-id.txt");
    let waited_run = sandbox.json([
        "--format",
        "json",
        "wait",
        "run",
        run_id.as_str(),
        "--state",
        "exited",
        "--timeout",
        "5s",
    ]);
    assert_eq!(waited_run["id"], run_id);
    assert_eq!(waited_run["state"], "exited");

    let subscribed_run = read_ndjson_until(&mut run_subscriber, |value| {
        value["name"] == "observer" && value["state"] == "exited"
    });
    assert_eq!(subscribed_run["id"], run_id);

    let events = sandbox.json([
        "event",
        "list",
        "--json",
        "--type",
        "query.request",
        "--source",
        "wait-e2e",
        "--correlation",
        "corr-wait",
    ]);
    assert_eq!(events.as_array().expect("events array").len(), 1);

    let triggers = sandbox.json([
        "trigger",
        "list",
        "--json",
        "--task",
        "observer",
        "--event",
        "query.request",
        "--outcome",
        "started",
        "--correlation",
        "corr-wait",
    ]);
    assert_eq!(triggers.as_array().expect("triggers array").len(), 1);

    let runs = sandbox.json([
        "run",
        "list",
        "--json",
        "--name",
        "observer",
        "--state",
        "exited",
        "--correlation",
        "corr-wait",
    ]);
    assert_eq!(runs.as_array().expect("runs array").len(), 1);
    assert_eq!(sandbox.read("correlation.txt"), "corr-wait");

    let _ = event_subscriber.kill();
    let _ = run_subscriber.kill();
    sandbox.ok(["down"]);
}

#[test]
fn adhoc_run_is_tracked_and_cancelable() {
    let sandbox = Sandbox::new("");
    sandbox.write_file("work/.keep", "");
    sandbox.ok(["up"]);

    let started = sandbox.json([
        "--format",
        "json",
        "run",
        "start",
        "--name",
        "adhoc-env",
        "--correlation",
        "corr-adhoc",
        "--cwd",
        "work",
        "--env",
        "EXTRA=from-env",
        "--json",
        r#"{"message":"payload"}"#,
        "--",
        "sh",
        "-c",
        "printf '%s|%s|%s|%s' \"$PWD\" \"$EXTRA\" \"$WHIPPLETREE_KIND\" \"$WHIPPLETREE_PAYLOAD_JSON\"; printf 'err-line' >&2",
    ]);
    let run_id = started["run_id"].as_str().expect("run id");

    let completed = sandbox.json([
        "--format",
        "json",
        "wait",
        "run",
        run_id,
        "--state",
        "exited",
        "--timeout",
        "5s",
    ]);
    assert_eq!(completed["origin"], "adhoc");
    assert_eq!(completed["name"], "adhoc-env");
    assert!(completed["event_id"].as_str().is_some());

    let logs = sandbox.json(["--format", "json", "run", "logs", run_id]);
    assert!(logs["stdout"]
        .as_str()
        .unwrap()
        .contains("work|from-env|adhoc|{\"message\":\"payload\"}"));
    assert!(logs["stderr"].as_str().unwrap().contains("err-line"));

    let events = sandbox.json([
        "event",
        "list",
        "--json",
        "--type",
        "adhoc.run.requested",
        "--correlation",
        "corr-adhoc",
    ]);
    assert_eq!(events.as_array().expect("events").len(), 1);
    assert_eq!(events[0]["source"], "adhoc");

    let runs = sandbox.json([
        "run",
        "list",
        "--json",
        "--origin",
        "adhoc",
        "--correlation",
        "corr-adhoc",
    ]);
    assert_eq!(runs.as_array().expect("runs").len(), 1);
    assert_eq!(runs[0]["id"], run_id);

    let cancel_started = sandbox.json([
        "--format",
        "json",
        "exec",
        "--name",
        "adhoc-cancel",
        "--",
        "sh",
        "-c",
        "sleep 30",
    ]);
    let cancel_run_id = cancel_started["run_id"].as_str().expect("cancel run id");
    sandbox.ok(["run", "cancel", cancel_run_id]);
    let cancelled = sandbox.json([
        "--format",
        "json",
        "wait",
        "run",
        cancel_run_id,
        "--state",
        "failed",
        "--timeout",
        "5s",
    ]);
    assert_eq!(cancelled["origin"], "adhoc");
    assert_eq!(cancelled["killed"], true);

    let timeout_started = sandbox.json([
        "--format",
        "json",
        "run",
        "start",
        "--name",
        "adhoc-timeout",
        "--timeout",
        "100ms",
        "--",
        "sh",
        "-c",
        "sleep 30",
    ]);
    let timeout_run_id = timeout_started["run_id"].as_str().expect("timeout run id");
    let timed_out = sandbox.json([
        "--format",
        "json",
        "wait",
        "run",
        timeout_run_id,
        "--state",
        "failed",
        "--timeout",
        "5s",
    ]);
    assert_eq!(timed_out["origin"], "adhoc");
    assert_eq!(timed_out["killed"], true);

    sandbox.ok(["down"]);
}

#[test]
fn sustained_event_watch_and_service_load_stays_responsive() {
    run_sustained_load_scenario(18, 6, 3, Duration::from_secs(8));
}

#[test]
#[ignore = "opt-in stress coverage; run with `cargo test -p whippletree-cli --test e2e -- --ignored sustained_stress_many_events_watch_changes_and_services`"]
fn sustained_stress_many_events_watch_changes_and_services() {
    run_sustained_load_scenario(90, 25, 10, Duration::from_secs(20));
}

fn run_sustained_load_scenario(
    event_count: usize,
    watch_count: usize,
    service_count: usize,
    timeout: Duration,
) {
    let mut config = String::from(
        r#"
        [[task]]
        name = "event-queue"
        on = "stress.event"
        run = "sh scripts/record-event.sh"

        [task.admission]
        when_busy = "queue_all"

        [[task]]
        name = "watch-burst"
        watch = ["watched/*.txt"]
        settle = "50ms"
        run = "sh scripts/record-watch.sh"

        [task.admission]
        when_busy = "queue_one"
        "#,
    );
    for index in 0..service_count {
        config.push_str(&format!(
            r#"

        [[service]]
        name = "svc-{index}"
        run = "sh scripts/service.sh"
        "#
        ));
    }

    let sandbox = Sandbox::new(&config);
    sandbox.write_file("watched/.keep", "");
    sandbox.write_script(
        "record-event.sh",
        r#"
        #!/bin/sh
        set -eu
        sleep 0.02
        printf '%s\n' "$WHIPPLETREE_EVENT_PAYLOAD_JSON" >> event-order.log
        "#,
    );
    sandbox.write_script(
        "record-watch.sh",
        r#"
        #!/bin/sh
        set -eu
        printf '%s\n' "$WHIPPLETREE_EVENT_PAYLOAD_JSON" >> watch-events.log
        "#,
    );
    sandbox.write_script(
        "service.sh",
        r#"
        #!/bin/sh
        set -eu
        printf '%s\n' "$WHIPPLETREE_NAME:$WHIPPLETREE_RUN_ID" >> services-started.log
        trap "exit 0" TERM INT
        while true; do sleep 1; done
        "#,
    );

    sandbox.ok(["up"]);
    wait_until_for(
        || running_service_count(&sandbox) == service_count,
        format!("{service_count} services to be running"),
        timeout,
    );

    for seq in 0..event_count {
        let payload = format!(r#"{{"seq":{seq}}}"#);
        sandbox.ok([
            "emit",
            "stress.event",
            "--source",
            "sustained-load-e2e",
            "--json",
            payload.as_str(),
        ]);
        if seq == event_count / 2 {
            let status = sandbox.json(["status", "--json"]);
            assert_eq!(status["services"].as_u64(), Some(service_count as u64));
            assert_eq!(status["tasks"].as_u64(), Some(2));
        }
    }

    for index in 0..watch_count {
        sandbox.write_file(&format!("watched/file-{index}.txt"), "changed\n");
    }
    wait_until_for(
        || count_events_matching(&sandbox, "file.changed", Some("watch")) > 0,
        "watch event to be recorded under load",
        timeout,
    );

    let service_zero = "svc-0";
    let service_one = if service_count > 1 { "svc-1" } else { "svc-0" };
    let original_restart_run = service_active_run_id(&sandbox, service_one);
    sandbox.ok(["service", "stop", service_zero]);
    wait_until_for(
        || service_is_stopped_override(&sandbox, service_zero),
        format!("{service_zero} to stop"),
        timeout,
    );
    sandbox.ok(["service", "start", service_zero]);
    wait_until_for(
        || service_active_run_id(&sandbox, service_zero).is_some(),
        format!("{service_zero} to restart after explicit start"),
        timeout,
    );
    sandbox.ok(["service", "restart", service_one]);
    wait_until_for(
        || {
            let current = service_active_run_id(&sandbox, service_one);
            current.is_some() && current != original_restart_run
        },
        format!("{service_one} to receive a replacement run"),
        timeout,
    );

    wait_until_for(
        || event_log_lines(&sandbox).len() == event_count,
        format!("{event_count} queued event runs to finish"),
        timeout,
    );
    let event_lines = event_log_lines(&sandbox);
    assert_eq!(event_lines.len(), event_count);
    for (expected, line) in event_lines.iter().enumerate() {
        let payload: Value = serde_json::from_str(line).expect("event payload json");
        assert_eq!(payload["seq"].as_u64(), Some(expected as u64));
    }

    assert_eq!(
        count_triggers_for_task(&sandbox, "event-queue", "started"),
        event_count
    );
    assert!(count_triggers_for_task(&sandbox, "watch-burst", "started") > 0);
    assert_eq!(
        count_events_matching(&sandbox, "stress.event", Some("event")),
        event_count
    );
    sandbox.ok(["status"]);
    sandbox.ok(["down"]);
}

fn event_log_lines(sandbox: &Sandbox) -> Vec<String> {
    let path = sandbox.root().join("event-order.log");
    match fs::read_to_string(path) {
        Ok(contents) => contents.lines().map(str::to_string).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("read event log: {error}"),
    }
}

fn running_service_count(sandbox: &Sandbox) -> usize {
    sandbox
        .json(["services", "--json"])
        .as_array()
        .unwrap()
        .iter()
        .filter(|service| service["state"] == "running")
        .count()
}

fn service_active_run_id(sandbox: &Sandbox, name: &str) -> Option<String> {
    sandbox
        .json(["services", "--json"])
        .as_array()
        .unwrap()
        .iter()
        .find(|service| service["name"] == name)
        .and_then(|service| service["active_run_id"].as_str())
        .map(str::to_string)
}

fn service_is_stopped_override(sandbox: &Sandbox, name: &str) -> bool {
    sandbox
        .json(["services", "--json"])
        .as_array()
        .unwrap()
        .iter()
        .find(|service| service["name"] == name)
        .map(|service| {
            service["stop_override"].as_bool() == Some(true)
                && service["active_run_id"].as_str().is_none()
        })
        .unwrap_or(false)
}

fn count_events_matching(sandbox: &Sandbox, event_type: &str, routing: Option<&str>) -> usize {
    sandbox
        .json(["events", "--json"])
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| {
            event["event_type"] == event_type
                && routing
                    .map(|expected| event["routing"] == expected)
                    .unwrap_or(true)
        })
        .count()
}

fn count_triggers_for_task(sandbox: &Sandbox, task_name: &str, outcome: &str) -> usize {
    sandbox
        .json(["triggers", "--json"])
        .as_array()
        .unwrap()
        .iter()
        .filter(|trigger| trigger["task_name"] == task_name && trigger["outcome"] == outcome)
        .count()
}

fn count_events(events: &Value, event_type: &str) -> usize {
    events
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["event_type"] == event_type)
        .count()
}

fn count_triggers(triggers: &Value, event_type: &str, outcome: Option<&str>) -> usize {
    triggers
        .as_array()
        .unwrap()
        .iter()
        .filter(|trigger| {
            trigger["event_type"] == event_type
                && outcome
                    .map(|expected| trigger["outcome"] == expected)
                    .unwrap_or(true)
        })
        .count()
}

fn has_trigger(triggers: &Value, event_type: &str, outcome: &str) -> bool {
    count_triggers(triggers, event_type, Some(outcome)) > 0
}

fn task_active(sandbox: &Sandbox, task_name: &str) -> bool {
    let tasks = sandbox.json(["--format", "json", "tasks"]);
    tasks.as_array().unwrap().iter().any(|task| {
        task["name"] == task_name && !task["active_run_ids"].as_array().unwrap().is_empty()
    })
}

fn file_line_count(sandbox: &Sandbox, relative_path: &str) -> usize {
    let path = sandbox.root().join(relative_path);
    fs::read_to_string(path)
        .map(|contents| contents.lines().count())
        .unwrap_or(0)
}

fn assert_contains_stderr(output: Output, needle: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(needle),
        "stderr did not contain {needle:?}\nstderr:\n{stderr}"
    );
}

fn wait_until(condition: impl FnMut() -> bool, description: impl AsRef<str>) {
    wait_until_for(condition, description, Duration::from_secs(5));
}

fn wait_until_for(
    mut condition: impl FnMut() -> bool,
    description: impl AsRef<str>,
    timeout: Duration,
) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {}", description.as_ref());
}

fn read_ndjson_until(child: &mut std::process::Child, predicate: impl Fn(&Value) -> bool) -> Value {
    let stdout = child.stdout.take().expect("subscriber stdout");
    let (sender, receiver) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let _ = sender.send(line.expect("read subscriber line"));
        }
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = receiver
            .recv_timeout(remaining.min(Duration::from_millis(250)))
            .unwrap_or_default();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line.trim()).expect("subscriber ndjson");
        if predicate(&value) {
            return value;
        }
    }
    panic!("timed out waiting for subscriber output");
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod script");
}
