use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
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
        fs::create_dir_all(root.path().join(".armature")).expect("create config dir");
        fs::create_dir_all(root.path().join("scripts")).expect("create scripts dir");
        fs::write(root.path().join(".armature/armature.toml"), config.trim())
            .expect("write config");

        Self {
            root,
            state,
            bin: PathBuf::from(env!("CARGO_BIN_EXE_armature")),
        }
    }

    fn write_script(&self, name: &str, contents: &str) {
        let path = self.root.path().join("scripts").join(name);
        fs::write(&path, contents.trim_start()).expect("write script");
        let mut permissions = fs::metadata(&path).expect("script metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("make script executable");
    }

    fn armature<I, S>(&self, args: I) -> Output
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
            .expect("run armature")
    }

    fn ok<I, S>(&self, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.armature(args);
        assert!(
            output.status.success(),
            "command failed\nstdout:\n{}\nstderr:\n{}",
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

    fn wait_for_file(&self, relative_path: &str) {
        let path = self.root.path().join(relative_path);
        wait_until(|| path.is_file(), format!("{} to exist", path.display()));
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = self.armature(["down"]);
    }
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
        printf "$ARMATURE_EVENT_TYPE" > event.txt
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
        sandbox.json(["--format", "json", "service", "show", "worker"])["name"],
        "worker"
    );

    sandbox.ok(["up"]);
    sandbox.wait_for_file("worker.txt");
    assert_eq!(
        sandbox.json(["--format", "json", "service", "list"])[0]["name"],
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
        "trigger to be recorded".to_string(),
    );
    let triggers = sandbox.json(["--format", "json", "triggers"]);
    let trigger_id = triggers[0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        sandbox.json(["--format", "json", "trigger", "show", &trigger_id])["id"],
        trigger_id
    );
}

fn wait_until(mut condition: impl FnMut() -> bool, description: String) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {description}");
}
