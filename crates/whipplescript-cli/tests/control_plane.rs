use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
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
        "clock-source.whip",
        "terminal-output-union.whip",
        "triage-flow.whip",
        "incident-router.whip",
        "scheduled-escalation.whip",
        "event-bridge.whip",
        "reusable-review-pattern.whip",
        "reusable-action-chain.whip",
        "exec-json-ingest.whip",
        "autoresearch-lite.whip",
        "gastown-lite.whip",
        "circuit-breaker.whip",
        "human-review.whip",
        "multi-agent-bounded-concurrency.whip",
        "messaging-demo.whip",
        "file-store-demo.whip",
        // `openclaw-lite.whip` and `package-memory.whip` import `memory`, which now
        // ships as an embedded std manifest (M5) — so they check clean in this
        // lock-free bundle, proving the embedded payoff for a real package.
        "openclaw-lite.whip",
        "package-memory.whip",
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

/// DR-0023: prove an `action`-expanded effect chain actually executes at runtime,
/// not just compiles. The inlined `tell -> after succeeds -> done + record` chain
/// `whip agents <workflow>` (std.agent introspection, DR-0015 declared tier)
/// lists each declared agent with its provider/profile/capacity and declared
/// skills/capabilities/tools, read from the compiled IR.
#[test]
fn agents_command_lists_declared_agents() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let example = example_path("clock-source.whip");
    let report = run_json(bin, &["--json", "agents", example.to_str().expect("utf-8")]);
    let agents = report
        .get("agents")
        .and_then(Value::as_array)
        .expect("agents array");
    let triager = agents
        .iter()
        .find(|agent| agent.get("name").and_then(Value::as_str) == Some("triager"))
        .expect("triager agent");
    assert_eq!(
        triager.get("provider").and_then(Value::as_str),
        Some("owned")
    );
    assert_eq!(
        triager.get("profile").and_then(Value::as_str),
        Some("repo-writer")
    );
    assert_eq!(triager.get("capacity").and_then(Value::as_u64), Some(1));
}

/// `whip providers <workflow>` aggregates every provider the program needs
/// (agents, channels, sources) with what references each.
#[test]
fn providers_command_aggregates_all_providers() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let example = example_path("clock-source.whip");
    let report = run_json(
        bin,
        &["--json", "providers", example.to_str().expect("utf-8")],
    );
    let providers = report
        .get("providers")
        .and_then(Value::as_array)
        .expect("providers array");
    let find = |name: &str| {
        providers
            .iter()
            .find(|p| p.get("provider").and_then(Value::as_str) == Some(name))
            .and_then(|p| p.get("used_by"))
            .and_then(Value::as_array)
            .map(|refs| {
                refs.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
    };
    // The `owned` provider is used by the `triager` agent; `clock` by the source.
    assert_eq!(find("owned"), Some(vec!["agent:triager".to_owned()]));
    assert_eq!(find("clock"), Some(vec!["source:daily_triage".to_owned()]));
}

/// `whip skills <workflow>` completes the introspection trio: every skill
/// declared across the program's agents, each with its declaring agents.
#[test]
fn skills_command_lists_declared_skills() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let example = example_path("revision-ticket-v1.whip");
    let report = run_json(bin, &["--json", "skills", example.to_str().expect("utf-8")]);
    let skills = report
        .get("skills")
        .and_then(Value::as_array)
        .expect("skills array");
    let author = skills
        .iter()
        .find(|s| s.get("skill").and_then(Value::as_str) == Some("whipplescript-author"))
        .and_then(|s| s.get("declared_by"))
        .and_then(Value::as_array)
        .map(|agents| {
            agents
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        });
    assert_eq!(author, Some(vec!["worker".to_owned()]));
}

/// (with a hygienic `turn__act0` binding) must run end to end under the fixture
/// provider: the tell effect completes, the seeded `ChangeRequest` is consumed,
/// and a `ReviewedChange` fact is recorded.
#[test]
fn action_expanded_chain_runs_end_to_end() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_store_path();
    let example = example_path("reusable-action-chain.whip");

    let dev = run_json(
        bin,
        &[
            "--store",
            store.to_str().expect("utf-8 store path"),
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

    let facts = run_json(
        bin,
        &[
            "--store",
            store.to_str().expect("utf-8 store path"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let fact_names: Vec<&str> = facts
        .as_array()
        .expect("facts array")
        .iter()
        .filter_map(|fact| fact.get("name").and_then(Value::as_str))
        .collect();
    assert!(
        fact_names.contains(&"ReviewedChange"),
        "inlined record ran: {fact_names:?}"
    );
    // `done item` consumed the seeded ChangeRequest fact.
    assert!(
        !fact_names.contains(&"ChangeRequest"),
        "inlined `done` consumed the input: {fact_names:?}"
    );

    let effects = run_json(
        bin,
        &[
            "--store",
            store.to_str().expect("utf-8 store path"),
            "--json",
            "effects",
            instance_id,
        ],
    );
    assert!(
        effects.as_array().expect("effects array").iter().any(|e| {
            e.get("kind").and_then(Value::as_str) == Some("agent.tell")
                && e.get("status").and_then(Value::as_str) == Some("completed")
        }),
        "inlined tell effect completed: {effects}"
    );

    let _ = fs::remove_file(store);
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

/// std.files `read` (spec/std-library/files.md): a `file store` declaration
/// scopes a root directory; `read text from <store> at <path>` settles to a
/// `file.read.completed` fact carrying the file content, which `after <binding>
/// succeeds` reacts to. Exercises the full vertical: parse -> lower -> claim ->
/// run (disk read) -> binding fact -> after-block -> workflow completion.
#[test]
fn dev_file_read_binds_content_and_completes() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let root = unique_temp_dir("file-read-root");
    fs::write(root.join("note.md"), "hello from disk").expect("seed file");
    let source_path = temp_workflow_path("file-read");
    fs::write(
        &source_path,
        format!(
            r#"
workflow FileRead

output result Result

class Result {{
  status string
}}

file store project_files {{
  root "{}"
}}

rule pick
  when started
=> {{
  read text from project_files at "note.md" as fileResult
  after fileResult succeeds as result {{
    complete result {{
      status "read-ok"
    }}
  }}
}}
"#,
            root.display()
        ),
    )
    .expect("write source");

    let store = store_path.to_str().expect("utf-8 temp path");
    let dev = run_json(
        bin,
        &[
            "--store",
            store,
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

    let status = run_json(bin, &["--store", store, "--json", "status", &instance_id]);
    assert_eq!(
        status.pointer("/instance/status").and_then(Value::as_str),
        Some("completed"),
        "file.read drives the workflow to completion: {status}"
    );

    let facts = run_json(bin, &["--store", store, "--json", "facts", &instance_id]);
    let read_fact = facts
        .as_array()
        .expect("facts")
        .iter()
        .find(|fact| fact.get("name").and_then(Value::as_str) == Some("file.read.completed"))
        .expect("file.read.completed fact present");
    assert_eq!(
        read_fact
            .pointer("/value/value/content")
            .and_then(Value::as_str),
        Some("hello from disk"),
        "the read binding carries the on-disk file content: {read_fact}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(root);
}

/// A `file` ingress source (mirroring the `clock` source, spec/std-time.md)
/// admits one durable signal fact per non-empty line of its `path`, mapping the
/// observation record `{ line, line_index, path }` onto the declared signal via
/// the author's `emit` clause. This exercises the full vertical: parse the
/// `path` clause -> lower an `is_file` source -> read the file on a worker pass
/// -> admit one `ingress.fed` signal per line -> a rule reacts and records a
/// `FedLine` fact. Admission is idempotent by (source, line ordinal): a second
/// worker pass over the same file re-admits nothing (append-only log semantics),
/// so the store never double-counts a line.
#[test]
fn dev_file_source_admits_one_signal_per_line_idempotently() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let root = unique_temp_dir("file-source-root");
    let inbox = root.join("inbox.txt");
    fs::write(&inbox, "first line\nsecond line\n").expect("seed inbox");
    let source_path = temp_workflow_path("file-source");
    fs::write(
        &source_path,
        format!(
            r#"
@service
workflow IngressFileSource

signal ingress.fed {{
  text string
  index int
}}

class FedLine {{
  text string
  index int
}}

source file as feed {{
  path "{}"

  observe as obs
  emit ingress.fed {{
    text obs.line
    index obs.line_index
  }}
}}

rule record_line
  when ingress.fed as f
=> {{
  record FedLine {{
    text f.text
    index f.index
  }}
}}
"#,
            inbox.display()
        ),
    )
    .expect("write source");

    let store = store_path.to_str().expect("utf-8 store path");
    let program = source_path.to_str().expect("utf-8 source path");
    let dev = run_json(
        bin,
        &[
            "--store",
            store,
            "--json",
            "dev",
            program,
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

    // One `ingress.fed` signal fact per line, carrying that line's text.
    let fed_texts = |instance: &str| -> Vec<String> {
        let facts = run_json(bin, &["--store", store, "--json", "facts", instance]);
        let mut rows: Vec<(i64, String)> = facts
            .as_array()
            .expect("facts array")
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("ingress.fed"))
            .map(|fact| {
                let index = fact
                    .pointer("/value/index")
                    .and_then(Value::as_i64)
                    .expect("fed fact carries a line index");
                let text = fact
                    .pointer("/value/text")
                    .and_then(Value::as_str)
                    .expect("fed fact carries the line text")
                    .to_owned();
                (index, text)
            })
            .collect();
        rows.sort_by_key(|(index, _)| *index);
        rows.into_iter().map(|(_, text)| text).collect()
    };

    assert_eq!(
        fed_texts(&instance_id),
        vec!["first line".to_owned(), "second line".to_owned()],
        "the file source admits one `ingress.fed` signal per non-empty line, in file order"
    );

    // The rule reacted to each admitted signal (one `FedLine` per line).
    let fed_line_count = |instance: &str| -> usize {
        let facts = run_json(bin, &["--store", store, "--json", "facts", instance]);
        facts
            .as_array()
            .expect("facts array")
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("FedLine"))
            .count()
    };
    assert_eq!(
        fed_line_count(&instance_id),
        2,
        "each admitted signal drove the reacting rule to record a FedLine fact"
    );

    // Idempotency: a second worker pass over the *same* instance and the *same*
    // file re-admits nothing. Without the (source, line ordinal) cursor this
    // re-attempt would hit the event log's UNIQUE(idempotency_key) constraint or
    // double the facts; instead the count is unchanged.
    let worker = run_json(
        bin,
        &[
            "--store",
            store,
            "--json",
            "worker",
            &instance_id,
            "--program",
            program,
            "--provider",
            "fixture",
        ],
    );
    assert_eq!(
        worker.get("file_lines_admitted").and_then(Value::as_u64),
        Some(0),
        "a re-read of an unchanged file admits no new lines: {worker}"
    );
    assert_eq!(
        fed_texts(&instance_id),
        vec!["first line".to_owned(), "second line".to_owned()],
        "re-running admits no duplicate signal facts (idempotent by line ordinal)"
    );
    assert_eq!(
        fed_line_count(&instance_id),
        2,
        "no duplicate FedLine facts after the idempotent re-read"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(root);
}

/// Spawns an in-process HTTP server on an ephemeral loopback port that answers
/// every GET with `200 OK` and the given JSON body (mirrors `spawn_otel_collector`
/// in soft_middle.rs, but serves rather than records, and loops so repeated
/// worker-pass GETs all get the same feed). Returns the bound port; the server
/// thread is detached and dies with the test process.
fn spawn_json_feed_server(body: String) -> u16 {
    use std::io::{Read as _, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind feed server");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            // Drain the request headers (a GET carries no body); we don't parse
            // them — every request gets the same feed.
            let mut scratch = [0u8; 2048];
            let _ = stream.read(&mut scratch);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    port
}

/// An `http` ingress source (mirroring the `file` source) GETs a URL returning a
/// JSON array and admits one durable signal fact per element, mapping the
/// observation record `{ item, item_index, url }` onto the declared signal via
/// the author's `emit` clause. This exercises the full vertical: parse the `url`
/// clause -> lower an `is_http` source -> GET the endpoint on a worker pass ->
/// admit one `ingress.ingested` signal per array element -> a rule reacts and
/// records an `IngestedItem` fact. Admission is idempotent by (source, element
/// ordinal): a second worker pass over the same (still-serving) feed re-admits
/// nothing (append-only feed semantics). Also asserts a dead endpoint does NOT
/// crash `dev`: a network error admits nothing rather than failing the worker.
#[test]
fn dev_http_source_admits_one_signal_per_array_element_idempotently() {
    let bin = env!("CARGO_BIN_EXE_whip");

    // The feed returns a JSON array of two elements. `emit`'s `text obs.item`
    // maps each element, re-stringified, onto the signal — so the admitted text
    // is exactly the element's JSON serialization (mirrors the runtime).
    let body = r#"["alpha","beta"]"#.to_owned();
    let expected_texts: Vec<String> = serde_json::from_str::<Value>(&body)
        .expect("valid feed json")
        .as_array()
        .expect("feed is an array")
        .iter()
        .map(|element| element.to_string())
        .collect();
    let port = spawn_json_feed_server(body);
    let feed_url = format!("http://127.0.0.1:{port}/feed.json");

    let store_path = temp_store_path();
    let source_path = temp_workflow_path("http-source");
    fs::write(
        &source_path,
        format!(
            r#"
@service
workflow IngressHttpSource

signal ingress.ingested {{
  text string
  index int
}}

class IngestedItem {{
  text string
  index int
}}

source http as feed {{
  url "{feed_url}"

  observe as obs
  emit ingress.ingested {{
    text obs.item
    index obs.item_index
  }}
}}

rule record_item
  when ingress.ingested as f
=> {{
  record IngestedItem {{
    text f.text
    index f.index
  }}
}}
"#
        ),
    )
    .expect("write source");

    let store = store_path.to_str().expect("utf-8 store path");
    let program = source_path.to_str().expect("utf-8 source path");
    let dev = run_json(
        bin,
        &[
            "--store",
            store,
            "--json",
            "dev",
            program,
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

    // One `ingress.ingested` signal fact per array element, in feed order.
    let ingested_texts = |instance: &str| -> Vec<String> {
        let facts = run_json(bin, &["--store", store, "--json", "facts", instance]);
        let mut rows: Vec<(i64, String)> = facts
            .as_array()
            .expect("facts array")
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("ingress.ingested"))
            .map(|fact| {
                let index = fact
                    .pointer("/value/index")
                    .and_then(Value::as_i64)
                    .expect("ingested fact carries an element index");
                let text = fact
                    .pointer("/value/text")
                    .and_then(Value::as_str)
                    .expect("ingested fact carries the element text")
                    .to_owned();
                (index, text)
            })
            .collect();
        rows.sort_by_key(|(index, _)| *index);
        rows.into_iter().map(|(_, text)| text).collect()
    };

    assert_eq!(
        ingested_texts(&instance_id),
        expected_texts,
        "the http source admits one `ingress.ingested` signal per array element, in feed order"
    );

    // The rule reacted to each admitted signal (one `IngestedItem` per element).
    let ingested_item_count = |instance: &str| -> usize {
        let facts = run_json(bin, &["--store", store, "--json", "facts", instance]);
        facts
            .as_array()
            .expect("facts array")
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("IngestedItem"))
            .count()
    };
    assert_eq!(
        ingested_item_count(&instance_id),
        2,
        "each admitted signal drove the reacting rule to record an IngestedItem fact"
    );

    // Idempotency: a second worker pass over the *same* instance re-polls the
    // *same* feed and re-admits nothing. Without the (source, element ordinal)
    // cursor this re-attempt would hit the event log's UNIQUE(idempotency_key)
    // constraint or double the facts; instead the count is unchanged.
    let worker = run_json(
        bin,
        &[
            "--store",
            store,
            "--json",
            "worker",
            &instance_id,
            "--program",
            program,
            "--provider",
            "fixture",
        ],
    );
    assert_eq!(
        worker.get("http_items_admitted").and_then(Value::as_u64),
        Some(0),
        "a re-poll of an unchanged feed admits no new elements: {worker}"
    );
    assert_eq!(
        ingested_texts(&instance_id),
        expected_texts,
        "re-polling admits no duplicate signal facts (idempotent by element ordinal)"
    );
    assert_eq!(
        ingested_item_count(&instance_id),
        2,
        "no duplicate IngestedItem facts after the idempotent re-poll"
    );

    // A network error is not a hard failure: a second workflow pointed at a dead
    // port (nothing listening) must let `dev` complete with nothing admitted,
    // never crash the worker.
    let dead_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind for dead port");
        let port = listener.local_addr().expect("addr").port();
        drop(listener); // close it so the port refuses connections
        port
    };
    let dead_store = temp_store_path();
    let dead_source = temp_workflow_path("http-source-dead");
    fs::write(
        &dead_source,
        format!(
            r#"
@service
workflow IngressHttpDead

signal ingress.ingested {{
  text string
}}

class IngestedItem {{
  text string
}}

source http as feed {{
  url "http://127.0.0.1:{dead_port}/feed.json"

  observe as obs
  emit ingress.ingested {{
    text obs.item
  }}
}}

rule record_item
  when ingress.ingested as f
=> {{
  record IngestedItem {{
    text f.text
  }}
}}
"#
        ),
    )
    .expect("write dead source");

    let dead_store_str = dead_store.to_str().expect("utf-8 dead store");
    let dead_program = dead_source.to_str().expect("utf-8 dead source");
    // `dev --until idle` completes (run_json asserts exit 0) despite the dead
    // endpoint; nothing is admitted.
    let dead_dev = run_json(
        bin,
        &[
            "--store",
            dead_store_str,
            "--json",
            "dev",
            dead_program,
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let dead_instance = dead_dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("dead instance id");
    let dead_facts = run_json(
        bin,
        &["--store", dead_store_str, "--json", "facts", dead_instance],
    );
    let dead_ingested = dead_facts
        .as_array()
        .expect("facts array")
        .iter()
        .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("ingress.ingested"))
        .count();
    assert_eq!(
        dead_ingested, 0,
        "a dead endpoint admits nothing and does not crash `dev`"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(dead_store);
    let _ = fs::remove_file(dead_source);
}

/// std.files `write` (spec/std-library/files.md): `write text to <store> at
/// <path> { body <expr> mode <mode> }` renders a body to disk through the real
/// worker, settling `file.write.completed`. The mode is enforced ("no silent
/// overwrite"): `create` on an existing file is an ordinary failure routed to
/// `after w fails`, leaving the file untouched. Exercises body resolution from
/// an `after` binding and the create/replace modes.
#[test]
fn dev_file_write_renders_body_and_enforces_mode() {
    let bin = env!("CARGO_BIN_EXE_whip");

    // 1) create succeeds: a fresh file is written with the resolved body.
    let create_store = temp_store_path();
    let create_root = unique_temp_dir("file-write-create-root");
    let create_src = temp_workflow_path("file-write-create");
    fs::write(
        &create_src,
        format!(
            r#"
workflow WriteCreate

output result Result

class Result {{
  status string
}}

file store out_files {{
  root "{}"
}}

rule pick
  when started
=> {{
  write text to out_files at "report.md" {{
    body "rendered body"
    mode create
  }} as written
  after written succeeds as result {{
    complete result {{
      status "wrote"
    }}
  }}
}}
"#,
            create_root.display()
        ),
    )
    .expect("write create source");
    let store = create_store.to_str().expect("utf-8 temp path");
    let dev = run_json(
        bin,
        &[
            "--store",
            store,
            "--json",
            "dev",
            create_src.to_str().expect("utf-8 source path"),
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
    let status = run_json(bin, &["--store", store, "--json", "status", &instance_id]);
    assert_eq!(
        status.pointer("/instance/status").and_then(Value::as_str),
        Some("completed"),
        "write create drives the workflow to completion: {status}"
    );
    assert_eq!(
        fs::read_to_string(create_root.join("report.md")).ok(),
        Some("rendered body".to_owned()),
        "the body is rendered to disk"
    );

    // 2) create on an existing file fails (no silent overwrite) and leaves the
    //    file untouched, routed to `after written fails`.
    let mv_store = temp_store_path();
    let mv_root = unique_temp_dir("file-write-mode-root");
    fs::write(mv_root.join("exists.md"), "preexisting").expect("seed existing file");
    let mv_src = temp_workflow_path("file-write-mode");
    fs::write(
        &mv_src,
        format!(
            r#"
workflow WriteMode

output result Result
failure error Stopped

class Result {{
  status string
}}

class Stopped {{
  reason string
}}

file store out_files {{
  root "{}"
}}

rule pick
  when started
=> {{
  write text to out_files at "exists.md" {{
    body "new content"
    mode create
  }} as written
  after written fails as err {{
    fail error {{
      reason "already exists"
    }}
  }}
}}
"#,
            mv_root.display()
        ),
    )
    .expect("write mode source");
    let mv_store_str = mv_store.to_str().expect("utf-8 temp path");
    let mv_dev = run_json(
        bin,
        &[
            "--store",
            mv_store_str,
            "--json",
            "dev",
            mv_src.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let mv_instance = mv_dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();
    let mv_status = run_json(
        bin,
        &["--store", mv_store_str, "--json", "status", &mv_instance],
    );
    assert_eq!(
        mv_status
            .pointer("/instance/status")
            .and_then(Value::as_str),
        Some("failed"),
        "create on an existing file fails the workflow: {mv_status}"
    );
    assert_eq!(
        fs::read_to_string(mv_root.join("exists.md")).ok(),
        Some("preexisting".to_owned()),
        "the existing file is not overwritten"
    );

    let _ = fs::remove_file(create_store);
    let _ = fs::remove_file(create_src);
    let _ = fs::remove_dir_all(create_root);
    let _ = fs::remove_file(mv_store);
    let _ = fs::remove_file(mv_src);
    let _ = fs::remove_dir_all(mv_root);
}

/// A `file store`'s `allow read [...]` policy narrows which paths a `read` may
/// touch (beyond root containment): a path matching a glob reads, a path inside
/// the root but outside the policy fails. An empty policy means any path in the
/// root.
#[test]
fn dev_file_store_allow_policy_scopes_read_paths() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let root = unique_temp_dir("file-allow-root");
    fs::create_dir_all(root.join("docs")).expect("docs dir");
    fs::create_dir_all(root.join("secret")).expect("secret dir");
    fs::write(root.join("docs/a.md"), "public").expect("seed allowed");
    fs::write(root.join("secret/b.md"), "private").expect("seed denied");
    let source_path = temp_workflow_path("file-allow");
    fs::write(
        &source_path,
        format!(
            r#"
workflow Allow

output result Result
failure error Stopped

class Result {{
  status string
}}

class Stopped {{
  reason string
}}

file store files {{
  root "{}"
  allow read ["docs/**"]
}}

rule pick
  when started
=> {{
  read text from files at "INPUT" as r
  after r succeeds as v {{
    complete result {{
      status "read"
    }}
  }}
  after r fails as e {{
    fail error {{
      reason "denied"
    }}
  }}
}}
"#,
            root.display()
        ),
    )
    .expect("write source");

    let status_for = |relative: &str| {
        let store_path = temp_store_path();
        let store = store_path.to_str().expect("utf-8 temp path").to_owned();
        let body = fs::read_to_string(&source_path).expect("read source");
        let scenario_src = temp_workflow_path("file-allow-run");
        fs::write(&scenario_src, body.replace("INPUT", relative)).expect("write scenario");
        let dev = run_json(
            bin,
            &[
                "--store",
                &store,
                "--json",
                "dev",
                scenario_src.to_str().expect("utf-8 source path"),
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
        let status = run_json(bin, &["--store", &store, "--json", "status", &instance_id]);
        let result = status
            .pointer("/instance/status")
            .and_then(Value::as_str)
            .unwrap_or("missing")
            .to_owned();
        let _ = fs::remove_file(store_path);
        let _ = fs::remove_file(scenario_src);
        result
    };

    assert_eq!(
        status_for("docs/a.md"),
        "completed",
        "a path matching `allow read` is read"
    );
    assert_eq!(
        status_for("secret/b.md"),
        "failed",
        "a path inside the root but outside the policy is refused"
    );

    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(root);
}

/// std.files `import jsonl` (spec/std-library/files.md): decode a structured file
/// into typed `<Schema>` facts via the fact-batch admission primitive — each row
/// becomes a fact that `when <Schema>` rules fan out over. The admission is
/// all-or-nothing: a row missing a required field fails the effect and admits no
/// fact for any row.
#[test]
fn dev_file_import_jsonl_admits_typed_rows_and_is_atomic() {
    let bin = env!("CARGO_BIN_EXE_whip");

    // 1) A well-formed file admits one typed fact per row; `when IssueRow` fans
    //    out over each. An @service workflow keeps running so the fan-out is not
    //    raced by a terminal.
    let ok_workflow = |path: &str| {
        format!(
            r#"
@service
workflow Importer

class IssueRow {{
  title string
  priority string
}}

class Seen {{
  title string
}}

file store data_files {{
  root "{path}"
}}

rule pick
  when started
=> {{
  import jsonl IssueRow from data_files at "issues.jsonl" as imported
}}

rule fan_out
  when IssueRow as row
=> {{
  record Seen {{
    title row.title
  }}
}}
"#
        )
    };

    let ok_root = unique_temp_dir("file-import-ok-root");
    fs::write(
        ok_root.join("issues.jsonl"),
        "{\"title\":\"Crash\",\"priority\":\"high\"}\n{\"title\":\"Typo\",\"priority\":\"low\"}\n",
    )
    .expect("seed jsonl");
    let ok_src = temp_workflow_path("file-import-ok");
    fs::write(&ok_src, ok_workflow(ok_root.to_str().expect("utf-8 root"))).expect("write source");
    let ok_store = temp_store_path();
    let ok_store_str = ok_store.to_str().expect("utf-8 store");
    let dev = run_json(
        bin,
        &[
            "--store",
            ok_store_str,
            "--json",
            "dev",
            ok_src.to_str().expect("utf-8 source"),
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
    let facts = run_json(
        bin,
        &["--store", ok_store_str, "--json", "facts", &instance_id],
    );
    let count_named = |name: &str| {
        facts
            .as_array()
            .expect("facts")
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some(name))
            .count()
    };
    assert_eq!(
        count_named("IssueRow"),
        2,
        "one typed fact per imported row"
    );
    assert_eq!(
        count_named("Seen"),
        2,
        "`when IssueRow` fans out over each row"
    );

    // 2) A row missing a required field fails the import and admits nothing.
    let bad_workflow = |path: &str| {
        format!(
            r#"
workflow Importer

output result Result
failure error Stopped

class Result {{
  status string
}}

class Stopped {{
  reason string
}}

class IssueRow {{
  title string
  priority string
}}

file store data_files {{
  root "{path}"
}}

rule pick
  when started
=> {{
  import jsonl IssueRow from data_files at "issues.jsonl" as imported
  after imported succeeds as r {{
    complete result {{
      status "imported"
    }}
  }}
  after imported fails as e {{
    fail error {{
      reason "bad rows"
    }}
  }}
}}
"#
        )
    };
    let bad_root = unique_temp_dir("file-import-bad-root");
    fs::write(
        bad_root.join("issues.jsonl"),
        "{\"title\":\"ok\",\"priority\":\"high\"}\n{\"title\":\"missing-priority\"}\n",
    )
    .expect("seed bad jsonl");
    let bad_src = temp_workflow_path("file-import-bad");
    fs::write(
        &bad_src,
        bad_workflow(bad_root.to_str().expect("utf-8 root")),
    )
    .expect("write source");
    let bad_store = temp_store_path();
    let bad_store_str = bad_store.to_str().expect("utf-8 store");
    let bad_dev = run_json(
        bin,
        &[
            "--store",
            bad_store_str,
            "--json",
            "dev",
            bad_src.to_str().expect("utf-8 source"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let bad_instance = bad_dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();
    let bad_status = run_json(
        bin,
        &["--store", bad_store_str, "--json", "status", &bad_instance],
    );
    assert_eq!(
        bad_status
            .pointer("/instance/status")
            .and_then(Value::as_str),
        Some("failed"),
        "an invalid row fails the import: {bad_status}"
    );
    let bad_facts = run_json(
        bin,
        &["--store", bad_store_str, "--json", "facts", &bad_instance],
    );
    assert_eq!(
        bad_facts
            .as_array()
            .expect("facts")
            .iter()
            .filter(|fact| fact.get("name").and_then(Value::as_str) == Some("IssueRow"))
            .count(),
        0,
        "all-or-nothing: no IssueRow facts are admitted when a row is invalid"
    );

    let _ = fs::remove_file(ok_store);
    let _ = fs::remove_file(ok_src);
    let _ = fs::remove_dir_all(ok_root);
    let _ = fs::remove_file(bad_store);
    let _ = fs::remove_file(bad_src);
    let _ = fs::remove_dir_all(bad_root);
}

/// std.files `export` (DR-0022): serialize a collection-valued projection
/// (`<Schema>` facts, optionally `where`-filtered, deterministically ordered) to
/// a structured file. Round-trips with `import`: load rows, then export the
/// filtered subset and confirm only the matching rows reach disk.
#[test]
fn dev_file_export_serializes_filtered_collection() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let root = unique_temp_dir("file-export-root");
    fs::write(
        root.join("in.jsonl"),
        "{\"id\":\"A\",\"status\":\"ready\"}\n{\"id\":\"B\",\"status\":\"blocked\"}\n{\"id\":\"C\",\"status\":\"ready\"}\n",
    )
    .expect("seed jsonl");
    let source_path = temp_workflow_path("file-export");
    fs::write(
        &source_path,
        format!(
            r#"
@service
workflow RoundTrip

class IssueRow {{
  id string @key
  status string
}}

file store fs {{
  root "{}"
}}

rule load
  when started
=> {{
  import jsonl IssueRow from fs at "in.jsonl" as imported
}}

rule dump
  when IssueRow as row where row.status == "ready"
=> {{
  export csv IssueRow to fs at "ready.csv" {{
    where status == "ready"
    mode upsert
  }} as exported
}}
"#,
            root.display()
        ),
    )
    .expect("write source");

    let store_path = temp_store_path();
    let store = store_path.to_str().expect("utf-8 store");
    run_json(
        bin,
        &[
            "--store",
            store,
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );

    let exported = fs::read_to_string(root.join("ready.csv")).expect("export file written");
    // Header + only the two `ready` rows (B/blocked is filtered out), in the
    // store's deterministic (name, key) order.
    assert_eq!(
        exported, "id,status\nA,ready\nC,ready\n",
        "got: {exported:?}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_dir_all(root);
}

/// The `file store` root is the scope boundary: a `..` path that would climb
/// out of the root is refused before any disk access, settling the read as
/// `file.read.failed` rather than reading a file outside the store.
#[test]
fn dev_file_read_refuses_path_escaping_store_root() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let root = unique_temp_dir("file-read-escape-root");
    // A secret living in the parent of the store root that `..` would reach.
    let secret = root
        .parent()
        .expect("temp root has a parent")
        .join(format!("whipplescript-secret-{}.txt", std::process::id()));
    fs::write(&secret, "TOPSECRET").expect("seed secret");
    let secret_name = secret
        .file_name()
        .and_then(|name| name.to_str())
        .expect("utf-8 secret name")
        .to_owned();
    let source_path = temp_workflow_path("file-read-escape");
    fs::write(
        &source_path,
        format!(
            r#"
workflow FileReadEscape

output result Result
failure error Stopped

class Result {{
  status string
}}

class Stopped {{
  reason string
}}

file store project_files {{
  root "{}"
}}

rule pick
  when started
=> {{
  read text from project_files at "../{secret_name}" as fileResult
  after fileResult fails as err {{
    fail error {{
      reason "blocked"
    }}
  }}
}}
"#,
            root.display()
        ),
    )
    .expect("write source");

    let store = store_path.to_str().expect("utf-8 temp path");
    let dev = run_json(
        bin,
        &[
            "--store",
            store,
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

    let status = run_json(bin, &["--store", store, "--json", "status", &instance_id]);
    assert_eq!(
        status.pointer("/instance/status").and_then(Value::as_str),
        Some("failed"),
        "an escaping path refuses the read and fails the workflow: {status}"
    );

    let facts = run_json(bin, &["--store", store, "--json", "facts", &instance_id]);
    let failed = facts
        .as_array()
        .expect("facts")
        .iter()
        .find(|fact| fact.get("name").and_then(Value::as_str) == Some("file.read.failed"))
        .expect("file.read.failed fact present");
    assert!(
        failed
            .pointer("/value/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("escapes")),
        "the failure names the scope escape: {failed}"
    );
    // The secret outside the root was never read into a binding.
    assert!(
        !facts
            .as_array()
            .expect("facts")
            .iter()
            .any(|fact| fact.get("name").and_then(Value::as_str) == Some("file.read.completed")),
        "no file.read.completed fact for an escaping path"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(secret);
    let _ = fs::remove_dir_all(root);
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

/// Flow carry-forward, end to end at runtime: triage-flow's pre-ask `tell` result
/// (`plan`) is carried across the `askHuman` boundary via flow state, so after
/// approval the `complete result { plan plan.summary }` resolves the carried turn
/// and the workflow completes with that summary in its output. If the binding were
/// not carried, `flowState.plan.summary` would not resolve and the output `plan`
/// would be empty/absent.
#[test]
fn flow_carries_pre_ask_binding_through_human_answer() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("triage-flow.whip");
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
    let inbox_item_id = inbox
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item.get("inbox_item_id"))
        .and_then(Value::as_str)
        .expect("inbox item id (ask ran)")
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
            "approve",
        ],
    );
    run_text(
        bin,
        &["--store", store, "step", &instance_id, "--program", source],
    );

    let log = run_json(bin, &["--store", store, "--json", "log", &instance_id]);
    let completed = log
        .as_array()
        .expect("events")
        .iter()
        .find(|event| event.get("event_type").and_then(Value::as_str) == Some("workflow.completed"))
        .expect("workflow completed after approval");
    // The output `plan` came from `flowState.plan.summary` — the carried pre-ask
    // turn. The fixture turn summary is "fixture completed".
    assert_eq!(
        completed
            .pointer("/payload/payload/plan")
            .and_then(Value::as_str),
        Some("fixture completed"),
        "carried pre-ask binding resolved in the completion output: {completed}"
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
    // openclaw-lite imports the external `memory` package, so check/dev require
    // the committed lock (its `source.path` resolves `packages/memory.json`
    // relative to the lock's directory, `examples/`).
    let lock = example_path("openclaw-lite.lock.json");
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
            "--package-lock",
            lock.to_str().expect("utf-8 lock path"),
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
fn dev_owned_harness_completes_turn_with_leaf_invariants() {
    // DR-0024 slice 1: `--provider owned` drives the brokered tool-use loop. The
    // turn must settle to exactly one `agent.turn.completed` fact (single
    // terminal), the in-turn observations must be EVIDENCE only (leaf-ness, I2),
    // and the `finish` rule must fire off the turn fact (drop-in compatibility).
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("owned-harness-demo.whip");
    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            example.to_str().expect("utf-8 example path"),
            "--provider",
            "owned",
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
    let fact_names: Vec<&str> = facts
        .iter()
        .filter_map(|fact| fact.get("name").and_then(Value::as_str))
        .collect();
    // Exactly one terminal turn fact (single terminal, layer 3).
    assert_eq!(
        fact_names
            .iter()
            .filter(|name| **name == "agent.turn.completed")
            .count(),
        1
    );
    // Leaf-ness (I2): no interior observation ever became a rule-matchable fact.
    assert!(
        !fact_names
            .iter()
            .any(|name| name.contains("brokered") || *name == "agent.turn.tool_requested"),
        "interior observation leaked into a fact: {fact_names:?}"
    );

    // The interior observations are recorded as evidence instead.
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
    let evidence = evidence
        .get("evidence")
        .and_then(Value::as_array)
        .expect("evidence array");
    assert!(
        evidence
            .iter()
            .any(|item| item.get("kind").and_then(Value::as_str)
                == Some("agent.turn.brokered.model_request")),
        "expected a model_request evidence row"
    );

    // context-assembly Phase 1 (Decision 5): every assembled context bundle is
    // recorded as a `context.bundle` evidence row before the turn. The owned
    // harness always assembles persona, guidelines, date, and cwd (tools too when
    // present), so there must be at least four such rows.
    let context_bundle_rows = evidence
        .iter()
        .filter(|item| item.get("kind").and_then(Value::as_str) == Some("context.bundle"))
        .count();
    assert!(
        context_bundle_rows >= 4,
        "expected one context.bundle evidence row per assembled bundle (>=4), got {context_bundle_rows}"
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
fn dev_native_provider_unavailable_blocks_effect_recoverably() {
    // DR-0020: a native agent effect whose provider binding is unavailable
    // (here: the `pi` sidecar cannot launch) is BLOCKED before provider execution
    // with a categorized reason — recoverable, not a terminal failure. The effect
    // never runs (no failed run, no agent.turn.failed event).
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

    let store_str = store_path.to_str().expect("utf-8 temp path");
    let output = Command::new(bin)
        .env(
            "WHIPPLESCRIPT_PI_RPC_COMMAND",
            "__whipplescript_missing_pi_rpc_command__",
        )
        .args([
            "--store",
            store_str,
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

    // The effect is blocked (recoverable), categorized provider_config — not failed.
    let effects = run_json(
        bin,
        &["--store", store_str, "--json", "effects", instance_id],
    );
    let tell = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("agent.tell"))
        .expect("agent.tell effect");
    assert_eq!(
        tell.get("status").and_then(Value::as_str),
        Some("blocked"),
        "effect should be blocked, not failed: {tell}"
    );
    assert_eq!(
        tell.pointer("/policy_block/category")
            .and_then(Value::as_str),
        Some("provider_health"),
        "block category: {tell}"
    );
    assert!(
        tell.pointer("/policy_block/detail")
            .and_then(Value::as_str)
            .is_some(),
        "block detail present: {tell}"
    );

    // No run was started and no failure was recorded (blocked before execution).
    let runs = run_json(bin, &["--store", store_str, "--json", "runs", instance_id]);
    assert!(
        !runs
            .as_array()
            .expect("runs array")
            .iter()
            .any(|run| run.get("provider").and_then(Value::as_str) == Some("pi")),
        "no pi run should start: {runs}"
    );
    let log = run_json(bin, &["--store", store_str, "--json", "log", instance_id]);
    let events = log.as_array().expect("event array");
    assert!(
        events.iter().any(|event| {
            event.get("event_type").and_then(Value::as_str) == Some("effect.blocked")
                && event.pointer("/payload/category").and_then(Value::as_str)
                    == Some("provider_health")
        }),
        "effect.blocked event with category recorded: {log}"
    );
    assert!(
        !events.iter().any(|event| {
            event.get("event_type").and_then(Value::as_str) == Some("agent.turn.failed")
        }),
        "the effect must not fail a turn: {log}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
}

#[test]
fn dev_provider_config_profile_allowlist_blocks_effect_recoverably() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let source_path = temp_workflow_path("provider-profile-allowlist");
    let config_path = temp_workflow_path("provider-profile-allowlist-config");
    fs::write(
        &source_path,
        r#"
workflow ProviderProfileAllowlist

harness runner: command

agent worker using runner {
  profile "repo-reader"
  capacity 1
}

rule start_work
  when started
  when worker is available
=> {
  tell worker "this command harness should be blocked by profile allow-list"
}
"#,
    )
    .expect("write provider profile allowlist workflow");
    fs::write(
        &config_path,
        json!({
            "providers": [
                {
                    "provider_id": "runner",
                    "provider_kind": "command",
                    "surface": "command",
                    "workspace_policy": "read_only",
                    "cancellation_depth": "none",
                    "artifact_policy": "metadata",
                    "profile_ids": ["repo-writer"],
                    "executable": "sh",
                    "args": ["-c", "echo should-not-run"]
                }
            ]
        })
        .to_string(),
    )
    .expect("write provider config");

    let store_str = store_path.to_str().expect("utf-8 temp path");
    let dev = run_json(
        bin,
        &[
            "--store",
            store_str,
            "--json",
            "dev",
            source_path.to_str().expect("utf-8 source path"),
            "--provider",
            "fixture",
            "--provider-config",
            config_path.to_str().expect("utf-8 config path"),
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
        &["--store", store_str, "--json", "effects", instance_id],
    );
    let tell = effects
        .as_array()
        .expect("effects array")
        .iter()
        .find(|effect| effect.get("kind").and_then(Value::as_str) == Some("agent.tell"))
        .expect("agent.tell effect");

    assert_eq!(tell.get("status").and_then(Value::as_str), Some("blocked"));
    assert_eq!(
        tell.pointer("/policy_block/category")
            .and_then(Value::as_str),
        Some("provider_config")
    );
    assert!(
        tell.pointer("/policy_block/detail")
            .and_then(Value::as_str)
            .is_some_and(|detail| detail.contains("does not allow profile `repo-reader`")),
        "block detail: {tell}"
    );
    assert!(
        run_json(bin, &["--store", store_str, "--json", "runs", instance_id])
            .as_array()
            .expect("runs array")
            .is_empty()
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(config_path);
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
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("schema.coerce.failed")));
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
workflow TrackerClaim

tracker backlog {
  provider builtin
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule seed
  when started
=> {
  file issue into backlog {
    title "Fix it"
    body "Please"
  }
}

rule start_item
  when backlog has ready issue as item
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
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("tracker.claim.completed")));
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("agent.turn.completed")));
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("tracker.finish.completed")));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(items_path);
    let _ = fs::remove_file(workflow_path);
}

/// Holder-lifetime release on cancel (spec/coordination.md principle 3): a
/// `lease ... until ttl` is held for the instance's lifetime, and an operator
/// `cancel` reaches a terminal — so the lease must be dropped immediately, not
/// left dangling until the TTL crash net. Regression for the bug where `cancel`
/// transitioned the instance to terminal via the kernel without releasing the
/// coordination leases it held (only rule-driven `complete`/`fail` released).
#[test]
fn cancel_releases_held_coordination_leases() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let coordination_path = temp_store_path();
    let workflow_path = temp_workflow_path("cancel-releases-lease");
    fs::write(
        &workflow_path,
        r#"
@service
workflow CancelReleasesLease

class Key { id string }
class Held { tag string }

output result Held

lease slot {
  key Held
  slots 1
  ttl 1h
}

rule seed
  when started
=> {
  record Key {
    id "only"
  }
}

rule grab
  when Key as k
=> {
  acquire slot for k.id until ttl as lease

  after lease held {
    record Held {
      tag k.id
    }
  }
  after lease contended {
    complete result {
      tag k.id
    }
  }
}
"#,
    )
    .expect("workflow writes");

    // Drive to idle: the instance acquires the fire-and-forget lease and stays
    // running (the `held` branch never terminates), so the lease is held.
    let dev_output = Command::new(bin)
        .env("WHIPPLESCRIPT_COORDINATION_STORE", &coordination_path)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ])
        .output()
        .expect("dev runs");
    assert!(
        dev_output.status.success(),
        "dev failed: {}",
        String::from_utf8_lossy(&dev_output.stderr)
    );
    let dev_stdout = String::from_utf8_lossy(&dev_output.stdout);
    let dev: Value = serde_json::from_str(&dev_stdout[dev_stdout.find('{').expect("dev json")..])
        .expect("dev json");
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();

    let leases_held = coordination_leases(bin, &coordination_path);
    assert_eq!(
        leases_held.len(),
        1,
        "the fire-and-forget lease should be held before cancel, got {leases_held:?}"
    );
    assert_eq!(
        leases_held[0].get("holder").and_then(Value::as_str),
        Some(instance_id.as_str())
    );

    let cancel = Command::new(bin)
        .env("WHIPPLESCRIPT_COORDINATION_STORE", &coordination_path)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "cancel",
            &instance_id,
        ])
        .output()
        .expect("cancel runs");
    assert!(
        cancel.status.success(),
        "cancel failed: {}",
        String::from_utf8_lossy(&cancel.stderr)
    );

    let leases_after = coordination_leases(bin, &coordination_path);
    assert!(
        leases_after.is_empty(),
        "cancel must release the holder's leases (spec/coordination.md principle 3), still held: {leases_after:?}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(coordination_path);
    let _ = fs::remove_file(workflow_path);
}

/// Read the workspace-scoped coordination lease table via `whip --json leases`
/// (it reads only `WHIPPLESCRIPT_COORDINATION_STORE`, not the run store).
fn coordination_leases(bin: &str, coordination_path: &Path) -> Vec<Value> {
    let output = Command::new(bin)
        .env("WHIPPLESCRIPT_COORDINATION_STORE", coordination_path)
        .args(["--json", "leases"])
        .output()
        .expect("leases runs");
    assert!(
        output.status.success(),
        "leases failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("leases stdout is utf-8");
    serde_json::from_str::<Value>(&text)
        .expect("leases json")
        .as_array()
        .expect("leases json array")
        .clone()
}

/// Holder-lifetime release of builtin-queue claims on cancel (spec/work-queues.md):
/// a `claim`ed item is held `in_progress` by the claiming instance, and builtin
/// claims have NO TTL backstop — so if that instance is cancelled before it
/// `finish`es, the item must return to `open` for another worker. Regression for
/// the bug where a cancelled instance's claim stayed `in_progress` forever.
#[test]
fn cancel_releases_held_queue_claims() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let items_path = temp_store_path();
    let workflow_path = temp_workflow_path("cancel-releases-claim");
    fs::write(
        &workflow_path,
        r#"
@service
workflow CancelReleasesClaim

tracker backlog {
  provider builtin
}

class Marker { note string }

rule seed
  when started
=> {
  file issue into backlog {
    title "work"
    body "do it"
  }
}

rule grab
  when backlog has ready issue as item
=> {
  claim item as lease

  after lease succeeds {
    record Marker {
      note "claimed but not finished"
    }
  }
}
"#,
    )
    .expect("workflow writes");

    // Drive to idle: the instance files an item, claims it, and stays running
    // without finishing — so the item is held `in_progress`.
    let dev_output = Command::new(bin)
        .env("WHIPPLESCRIPT_ITEMS_STORE", &items_path)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ])
        .output()
        .expect("dev runs");
    assert!(
        dev_output.status.success(),
        "dev failed: {}",
        String::from_utf8_lossy(&dev_output.stderr)
    );
    let dev_stdout = String::from_utf8_lossy(&dev_output.stdout);
    let dev: Value = serde_json::from_str(&dev_stdout[dev_stdout.find('{').expect("dev json")..])
        .expect("dev json");
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();

    let claimed = tracker_items(bin, &items_path);
    assert_eq!(claimed.len(), 1, "expected one filed item, got {claimed:?}");
    assert_eq!(
        claimed[0].get("status").and_then(Value::as_str),
        Some("in_progress")
    );
    assert_eq!(
        claimed[0].get("claimed_by").and_then(Value::as_str),
        Some(instance_id.as_str())
    );

    let cancel = Command::new(bin)
        .env("WHIPPLESCRIPT_ITEMS_STORE", &items_path)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "cancel",
            &instance_id,
        ])
        .output()
        .expect("cancel runs");
    assert!(
        cancel.status.success(),
        "cancel failed: {}",
        String::from_utf8_lossy(&cancel.stderr)
    );

    let after = tracker_items(bin, &items_path);
    assert_eq!(
        after[0].get("status").and_then(Value::as_str),
        Some("open"),
        "cancel must return the claimed item to the backlog (spec/work-queues.md): {after:?}"
    );
    assert!(
        after[0]
            .get("claimed_by")
            .map(Value::is_null)
            .unwrap_or(true),
        "released item must have no claimant: {after:?}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(items_path);
    let _ = fs::remove_file(workflow_path);
}

/// Read the workspace-scoped builtin tracker via `whip --json items` (reads only
/// `WHIPPLESCRIPT_ITEMS_STORE`, not the run store).
fn tracker_items(bin: &str, items_path: &Path) -> Vec<Value> {
    let output = Command::new(bin)
        .env("WHIPPLESCRIPT_ITEMS_STORE", items_path)
        .args(["--json", "items"])
        .output()
        .expect("items runs");
    assert!(
        output.status.success(),
        "items failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("items stdout is utf-8");
    serde_json::from_str::<Value>(&text)
        .expect("items json")
        .as_array()
        .expect("items json array")
        .clone()
}

/// Holder-lifetime cleanup of pending human asks on cancel: a cancelled
/// instance's `pending` inbox item is moot, so it must leave the inbox and
/// become unanswerable — otherwise an operator could waste a decision on a dead
/// instance. Regression for the bug where cancel left the ask `pending` and
/// still answerable.
#[test]
fn cancel_retires_pending_human_asks() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let example = example_path("human-review.whip");

    // Drive to idle: the workflow issues a human ask and waits, so one inbox
    // item is pending.
    let dev_output = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "dev",
            example.to_str().expect("utf-8 example path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ])
        .output()
        .expect("dev runs");
    assert!(
        dev_output.status.success(),
        "dev failed: {}",
        String::from_utf8_lossy(&dev_output.stderr)
    );
    let dev_stdout = String::from_utf8_lossy(&dev_output.stdout);
    let dev: Value = serde_json::from_str(&dev_stdout[dev_stdout.find('{').expect("dev json")..])
        .expect("dev json");
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();

    let pending = pending_inbox(bin, &store_path);
    assert_eq!(
        pending.len(),
        1,
        "expected one pending ask, got {pending:?}"
    );
    let item_id = pending[0]
        .get("inbox_item_id")
        .and_then(Value::as_str)
        .expect("inbox item id")
        .to_owned();

    let cancel = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "cancel",
            &instance_id,
        ])
        .output()
        .expect("cancel runs");
    assert!(
        cancel.status.success(),
        "cancel failed: {}",
        String::from_utf8_lossy(&cancel.stderr)
    );

    assert!(
        pending_inbox(bin, &store_path).is_empty(),
        "cancel must retire the dead instance's pending asks"
    );

    // The ask is no longer answerable — answering a cancelled instance's ask
    // must fail rather than wasting a human decision on a dead instance.
    let answer = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "inbox",
            "answer",
            &item_id,
            "--choice",
            "accept",
        ])
        .output()
        .expect("answer runs");
    assert!(
        !answer.status.success(),
        "answering a cancelled instance's ask must fail"
    );

    let _ = fs::remove_file(store_path);
}

/// Read pending human asks via `whip --json inbox` (run store).
fn pending_inbox(bin: &str, store_path: &Path) -> Vec<Value> {
    let output = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "inbox",
        ])
        .output()
        .expect("inbox runs");
    assert!(
        output.status.success(),
        "inbox failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("inbox stdout is utf-8");
    serde_json::from_str::<Value>(&text)
        .expect("inbox json")
        .as_array()
        .expect("inbox json array")
        .clone()
}

/// A failing child invocation must drive the parent's `after child fails`
/// branch, not its `after child succeeds` branch. Regression for the bug where a
/// failed `invoke` emitted a `workflow.invoke.completed` terminal marker that the
/// `succeeds` predicate matched, firing the success branch and binding its
/// success value (`r.value`) to the failure payload — the parent's rule lowering
/// then failed on the unresolvable `r.value` and the parent hung in `running`.
#[test]
fn failed_child_invocation_drives_parent_failure_branch() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("invoke-child-fails");
    fs::write(
        &workflow_path,
        r#"
workflow Parent {
  input task Task
  output result PResult
  failure error PFail

  class Task { id string }
  class PResult { echo string }
  class PFail { reason string }

  rule go
    when Task as t
  => {
    invoke Child { seed { id t.id } } as child

    after child succeeds as r {
      complete result {
        echo r.value
      }
    }
    after child fails as f {
      fail error {
        reason f.reason
      }
    }
  }
}

workflow Child {
  input seed Seed
  output result CResult
  failure error CFail

  class Seed { id string }
  class CResult { value string }
  class CFail { reason string }

  rule boom
    when Seed as s
  => {
    fail error {
      reason "child boom"
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
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
            "--input",
            r#"{"task":{"id":"abc"}}"#,
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    let parent_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();

    let instances = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "instances",
        ],
    );
    let parent = instances
        .as_array()
        .expect("instances array")
        .iter()
        .find(|i| i.get("instance_id").and_then(Value::as_str) == Some(parent_id.as_str()))
        .expect("parent instance");
    // The core regression: the parent reaches its `fail` terminal. Before the
    // fix the success branch fired on the failed child's `.completed` marker and
    // the rule lowering failed on `r.value`, leaving the parent stuck `running`.
    assert_eq!(
        parent.get("status").and_then(Value::as_str),
        Some("failed"),
        "parent must reach the failure branch, not hang: {parent:?}"
    );

    // The child's failure reason reached the parent (the invoke terminal fact).
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "facts",
            &parent_id,
        ],
    );
    let invoke_failed = facts
        .as_array()
        .expect("facts array")
        .iter()
        .find(|f| f.get("name").and_then(Value::as_str) == Some("workflow.invoke.failed"))
        .expect("invoke failure fact");
    assert_eq!(
        invoke_failed
            .pointer("/value/value/reason")
            .and_then(Value::as_str),
        Some("child boom"),
        "child failure reason should reach the parent: {invoke_failed:?}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

/// `case` over a `bool` field selects the matching branch at runtime: with
/// `ready true` the `true` branch fires and records `Picked { which "yes" }`,
/// and the workflow's `assert` confirms exactly that fact landed.
#[test]
fn dev_bool_case_selects_matching_branch() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("bool-case-runtime");
    fs::write(
        &workflow_path,
        r#"
workflow BoolRoute

class Flag { ready bool }
class Picked { which string }

assert count(Picked where which == "yes") == 1

rule seed
  when started
=> {
  record Flag {
    ready true
  }
}

rule route
  when Flag as f
=> {
  case f.ready {
    true => {
      record Picked {
        which "yes"
      }
    }
    false => {
      record Picked {
        which "no"
      }
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
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    assert!(
        dev.get("assertions")
            .and_then(Value::as_array)
            .expect("assertions")
            .iter()
            .all(|a| a.get("passed").and_then(Value::as_bool) == Some(true)),
        "bool-case assertion should pass: {dev:?}"
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "facts",
            &instance_id,
        ],
    );
    let picked = facts
        .as_array()
        .expect("facts array")
        .iter()
        .find(|f| f.get("name").and_then(Value::as_str) == Some("Picked"))
        .expect("Picked fact");
    assert_eq!(
        picked.pointer("/value/which").and_then(Value::as_str),
        Some("yes"),
        "the `true` branch should fire: {picked:?}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

/// Regression: a `case` branch whose body is on a SINGLE line
/// (`pattern => { record X { ... } }`) must fire at runtime. The case-block
/// parser previously forced branch depth >= 1 and collected only the FOLLOWING
/// lines, silently dropping a single-line branch's inline body — so the branch
/// never materialized (a check/runtime divergence that `whip fmt` did NOT fix,
/// since fmt leaves case branches single-line).
#[test]
fn dev_single_line_case_branch_body_fires() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("single-line-case");
    fs::write(
        &workflow_path,
        r#"
workflow SingleLineCase

class Flag { ready bool }
class Picked { which string }

assert count(Picked where which == "no") == 1

rule seed
  when started
=> {
  record Flag {
    ready false
  }
}

rule route
  when Flag as f
=> {
  case f.ready {
    true => { record Picked { which "yes" } }
    false => { record Picked { which "no" } }
  }
}
"#,
    )
    .expect("workflow writes");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--provider",
            "fixture",
            "--until",
            "idle",
        ],
    );
    assert!(
        dev.get("assertions")
            .and_then(Value::as_array)
            .expect("assertions")
            .iter()
            .all(|a| a.get("passed").and_then(Value::as_bool) == Some(true)),
        "single-line case branch should fire: {dev:?}"
    );
    let instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id")
        .to_owned();
    let facts = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "facts",
            &instance_id,
        ],
    );
    let picked = facts
        .as_array()
        .expect("facts array")
        .iter()
        .find(|f| f.get("name").and_then(Value::as_str) == Some("Picked"))
        .expect("Picked fact");
    assert_eq!(
        picked.pointer("/value/which").and_then(Value::as_str),
        Some("no"),
        "the `false` single-line branch should fire: {picked:?}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

/// The fixture coerce synthesizes its result from the DECLARED output schema
/// (union field -> first variant) for any schema not in the hand-tuned table, so
/// a coerce example runs under `--provider fixture` without a hardcoded
/// placeholder. Here `CustomVerdict.decision` is generated as `"alpha"` (first
/// variant), the `case` selects it, and the workflow completes.
#[test]
fn dev_fixture_coerce_synthesizes_result_from_declared_schema() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("schema-gen-coerce");
    fs::write(
        &workflow_path,
        r#"
workflow SchemaGen

output result R
class R { v string }
class CustomVerdict { decision "alpha" | "beta"  note string }

coerce judge(x string) -> CustomVerdict {
  prompt "{{ x }}"
}

class Seed { x string }
table seeds as Seed [
  {
    x "go"
  }
]

rule j
  when Seed as s
=> {
  coerce judge(s.x) as verdict
  after verdict succeeds as v {
    case v.decision {
      "alpha" => {
        complete result {
          v "alpha"
        }
      }
      "beta" => {
        complete result {
          v "beta"
        }
      }
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
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
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
    let instances = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "instances",
        ],
    );
    let inst = instances
        .as_array()
        .expect("instances")
        .iter()
        .find(|i| i.get("instance_id").and_then(Value::as_str) == Some(instance_id.as_str()))
        .expect("instance");
    assert_eq!(
        inst.get("status").and_then(Value::as_str),
        Some("completed"),
        "schema-synthesized coerce result should resolve `v.decision` and complete: {inst:?}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_inline_decide_result_flows_to_after_case() {
    // The original docs example: an inline `decide -> { fixed bool } as v`
    // synthesizes a hygienic anonymous result class so `after v succeeds as r`
    // resolves `r.fixed` for a `bool` `case` — exactly like a named coerce.
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("inline-decide-case");
    fs::write(
        &workflow_path,
        r#"
workflow InlineDecide

output result Decision
class Decision { label string }

class Seed { x string }
table seeds as Seed [
  {
    x "go"
  }
]

rule d
  when Seed as s
=> {
  decide "is it fixed?" -> { fixed bool } as v
  after v succeeds as r {
    case r.fixed {
      true => {
        complete result {
          label "fixed"
        }
      }
      false => {
        complete result {
          label "not-fixed"
        }
      }
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
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
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
    let instances = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 store path"),
            "--json",
            "instances",
        ],
    );
    let inst = instances
        .as_array()
        .expect("instances")
        .iter()
        .find(|i| i.get("instance_id").and_then(Value::as_str) == Some(instance_id.as_str()))
        .expect("instance");
    assert_eq!(
        inst.get("status").and_then(Value::as_str),
        Some("completed"),
        "inline decide result should resolve `r.fixed` and a bool case branch should complete: {inst:?}"
    );

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
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("schema.coerce.succeeded")));
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
    let lock_path = temp_workflow_path("capability-call-lock").with_extension("json");
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
  recall project_memory for item as context

  after context succeeds {
    tell worker """
    Use the recalled context for {{ item.title }}.
    """
  }
}
"#,
    )
    .expect("workflow writes");
    let memory_manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packages/memory.json");
    // Portable locks record `source.path` relative to the lock directory, so the
    // manifest must live under that directory. Co-locate a copy next to the lock.
    let manifest_copy = temp_workflow_path("capability-call-manifest").with_extension("json");
    fs::copy(&memory_manifest, &manifest_copy).expect("copy manifest beside lock");
    run_text(
        bin,
        &[
            "package",
            "lock",
            "--output",
            lock_path.to_str().expect("utf-8 lock path"),
            manifest_copy.to_str().expect("utf-8 manifest path"),
        ],
    );

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
            "--package-lock",
            lock_path.to_str().expect("utf-8 lock path"),
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
    let _ = fs::remove_file(lock_path);
}

/// `learn ... into <pool>` then `recall <pool> ...` round-trips through the
/// file-backed `MemoryCapabilityProvider` (selected by the `memory-provider`
/// binding): the recalled `MemoryContext` must contain the learned item. This is
/// the first per-capability real provider — recall is no longer the fixture.
#[test]
fn memory_roundtrip_recalls_the_learned_item() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("memory-roundtrip");
    let store_path = dir.join("store.db");
    let workflow_src =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/memory-roundtrip.whip");
    let workflow_path = dir.join("wf.whip");
    fs::copy(&workflow_src, &workflow_path).expect("copy roundtrip example");
    let memory_manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packages/memory.json");
    let manifest_copy = dir.join("memory.json");
    fs::copy(&memory_manifest, &manifest_copy).expect("copy manifest beside lock");
    let lock_path = dir.join("whip.lock");
    run_text(
        bin,
        &[
            "package",
            "lock",
            "--output",
            lock_path.to_str().expect("utf-8 lock"),
            manifest_copy.to_str().expect("utf-8 manifest"),
        ],
    );

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 store"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow"),
            "--provider",
            "fixture",
            "--until",
            "idle",
            "--package-lock",
            lock_path.to_str().expect("utf-8 lock"),
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
            store_path.to_str().expect("utf-8 store"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");

    // The `learn` write settled successfully (memory.write capability).
    assert!(
        facts.iter().any(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("capability.call.succeeded")
                && fact.pointer("/value/target").and_then(Value::as_str) == Some("memory.write")
        }),
        "learn should settle a memory.write success fact"
    );

    // The `recall` read back a real MemoryContext containing the learned item.
    let context = facts
        .iter()
        .find(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("capability.call.succeeded")
                && fact.pointer("/value/target").and_then(Value::as_str) == Some("memory.query")
        })
        .expect("recall should settle a memory.query success fact");
    let memory = context
        .pointer("/value/value")
        .expect("recall context value");
    assert_eq!(
        memory.get("pool").and_then(Value::as_str),
        Some("project_memory"),
        "context reports its pool"
    );
    assert_eq!(
        memory.get("count").and_then(Value::as_u64),
        Some(1),
        "exactly the one learned item is recalled"
    );
    let items = memory
        .get("items")
        .and_then(Value::as_array)
        .expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].get("note").and_then(Value::as_str),
        Some("remember alpha"),
        "the recalled item carries the learned note"
    );
    assert_eq!(
        items[0].get("pool").and_then(Value::as_str),
        Some("project_memory")
    );

    let _ = fs::remove_dir_all(&dir);
}

/// M5 embedded-manifest payoff: the same `learn`/`recall` round-trip runs with NO
/// `--package-lock` and no `whip.lock` anywhere. `memory` ships compiled into the
/// binary, so `check` passes and `dev` executes the real file-backed provider —
/// the recalled `MemoryContext` still carries the learned item. This is the proof
/// that the embedded manifest removes the lock requirement for a real package.
#[test]
fn memory_roundtrip_without_a_lock_uses_the_embedded_manifest() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("memory-roundtrip-no-lock");
    let store_path = dir.join("store.db");
    let workflow_src =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/memory-roundtrip.whip");
    let workflow_path = dir.join("wf.whip");
    fs::copy(&workflow_src, &workflow_path).expect("copy roundtrip example");

    // `check` resolves `use memory` + the `recall`/`learn` constructs from the
    // embedded manifest — no lock present, no `--package-lock` flag.
    let checked = Command::new(bin)
        .args(["check", workflow_path.to_str().expect("utf-8 workflow")])
        .output()
        .expect("check runs");
    assert!(
        checked.status.success(),
        "check must pass with no lock via the embedded `memory` manifest\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );

    // `dev` runs with no `--package-lock`: the embedded manifest seeds the store's
    // `memory.query`/`memory.write` providers + bindings, so the round-trip runs.
    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 store"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow"),
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
            store_path.to_str().expect("utf-8 store"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");

    // The `learn` write settled successfully (memory.write capability).
    assert!(
        facts.iter().any(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("capability.call.succeeded")
                && fact.pointer("/value/target").and_then(Value::as_str) == Some("memory.write")
        }),
        "learn should settle a memory.write success fact without a lock"
    );

    // The `recall` read back a real MemoryContext containing the learned item.
    let context = facts
        .iter()
        .find(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("capability.call.succeeded")
                && fact.pointer("/value/target").and_then(Value::as_str) == Some("memory.query")
        })
        .expect("recall should settle a memory.query success fact without a lock");
    let memory = context
        .pointer("/value/value")
        .expect("recall context value");
    assert_eq!(
        memory.get("count").and_then(Value::as_u64),
        Some(1),
        "exactly the one learned item is recalled"
    );
    let items = memory
        .get("items")
        .and_then(Value::as_array)
        .expect("items array");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].get("note").and_then(Value::as_str),
        Some("remember alpha"),
        "the recalled item carries the learned note"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// MEM-1 pool declaration end-to-end: the `examples/memory-pool-demo.whip` demo
/// declares `memory pool project_memory { context limit 8 }`, `check` renders the
/// pool in its `memory_pools` snapshot, and `dev` runs the `learn`/`recall`
/// against the declared pool to completion (lock-free, embedded manifest).
#[test]
fn memory_pool_declaration_demo_checks_and_recalls() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("memory-pool-demo");
    let store_path = dir.join("store.db");
    let workflow_src =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/memory-pool-demo.whip");
    let workflow_path = dir.join("wf.whip");
    fs::copy(&workflow_src, &workflow_path).expect("copy memory-pool demo");

    // `check` resolves `use memory` from the embedded manifest (no lock) and
    // renders the declared pool + its context limit in the `.ir` snapshot.
    let checked = Command::new(bin)
        .args(["check", workflow_path.to_str().expect("utf-8 workflow")])
        .output()
        .expect("check runs");
    assert!(
        checked.status.success(),
        "check must pass for the memory-pool demo\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );
    let snapshot = String::from_utf8_lossy(&checked.stdout);
    assert!(
        snapshot.contains("memory_pools"),
        "check snapshot should list memory pools:\n{snapshot}"
    );
    assert!(
        snapshot.contains("memory pool project_memory"),
        "check snapshot should name the declared pool:\n{snapshot}"
    );
    assert!(
        snapshot.contains("context limit 8"),
        "check snapshot should render the pool's context limit:\n{snapshot}"
    );

    // `dev` runs the `learn`/`recall` against the declared pool to completion.
    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 store"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow"),
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
            store_path.to_str().expect("utf-8 store"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");

    // The `learn` (memory.write) and `recall` (memory.query) both settled against
    // the declared pool.
    assert!(
        facts.iter().any(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("capability.call.succeeded")
                && fact.pointer("/value/target").and_then(Value::as_str) == Some("memory.write")
        }),
        "learn should settle a memory.write success fact"
    );
    let recall = facts
        .iter()
        .find(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("capability.call.succeeded")
                && fact.pointer("/value/target").and_then(Value::as_str) == Some("memory.query")
        })
        .expect("recall should settle a memory.query success fact");
    assert_eq!(
        recall.pointer("/value/value/pool").and_then(Value::as_str),
        Some("project_memory"),
        "the recall context reports the declared pool"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// MEM-6 `curate` end-to-end, lock-free (embedded manifest). A workflow learns two
/// duplicate items (same `source`/`note`) and one distinct item into a pool, then
/// `curate`s it, then `recall`s. The three learns and the curate/recall are chained
/// across rules by marker facts so they execute strictly in order (a single rule's
/// nested effects are not serialized by the runtime). The curation result reports the
/// duplicate removed, and the post-curate recall returns exactly the deduped set.
#[test]
fn memory_curate_dedupes_the_pool_without_a_lock() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("memory-curate-no-lock");
    let store_path = dir.join("store.db");
    let workflow_src =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/memory-curate.whip");
    let workflow_path = dir.join("wf.whip");
    fs::copy(&workflow_src, &workflow_path).expect("copy curate example");

    // `check` resolves `use memory` + the `learn`/`curate`/`recall` constructs from
    // the embedded manifest — no lock present, no `--package-lock` flag.
    let checked = Command::new(bin)
        .args(["check", workflow_path.to_str().expect("utf-8 workflow")])
        .output()
        .expect("check runs");
    assert!(
        checked.status.success(),
        "check must pass with no lock via the embedded `memory` manifest\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr)
    );

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 store"),
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow"),
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
            store_path.to_str().expect("utf-8 store"),
            "--json",
            "facts",
            instance_id,
        ],
    );
    let facts = facts.as_array().expect("facts array");

    // All three `learn` writes settled (memory.write capability).
    let writes = facts
        .iter()
        .filter(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("capability.call.succeeded")
                && fact.pointer("/value/target").and_then(Value::as_str) == Some("memory.write")
        })
        .count();
    assert_eq!(writes, 3, "all three learns should settle a memory.write");

    // The `curate` maintenance op settled and reports the duplicate removed.
    let curation = facts
        .iter()
        .find(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("capability.call.succeeded")
                && fact.pointer("/value/target").and_then(Value::as_str) == Some("memory.curate")
        })
        .expect("curate should settle a memory.curate success fact");
    let result = curation
        .pointer("/value/value")
        .expect("curation result value");
    assert_eq!(
        result.get("pool").and_then(Value::as_str),
        Some("project_memory"),
        "curation result reports its pool"
    );
    let removed = result
        .get("removed")
        .and_then(Value::as_u64)
        .expect("removed count");
    assert!(
        removed >= 1,
        "curate must drop at least the one duplicate (removed = {removed})"
    );
    assert_eq!(
        result.get("kept").and_then(Value::as_u64),
        Some(2),
        "curate keeps the deduped set: one `dup` + one `distinct`"
    );

    // The post-curate `recall` returns exactly the deduped set (no duplicate `dup`).
    let context = facts
        .iter()
        .find(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("capability.call.succeeded")
                && fact.pointer("/value/target").and_then(Value::as_str) == Some("memory.query")
        })
        .expect("recall should settle a memory.query success fact");
    let memory = context
        .pointer("/value/value")
        .expect("recall context value");
    assert_eq!(
        memory.get("count").and_then(Value::as_u64),
        Some(2),
        "the deduped pool recalls exactly two entries"
    );
    let items = memory
        .get("items")
        .and_then(Value::as_array)
        .expect("items array");
    let mut notes: Vec<&str> = items
        .iter()
        .filter_map(|item| item.get("note").and_then(Value::as_str))
        .collect();
    notes.sort_unstable();
    assert_eq!(
        notes,
        vec!["distinct", "dup"],
        "the surviving entries are one `dup` (the deduped duplicate) and the `distinct` one"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Create a unique temp directory tagged with `label`.
fn unique_temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "whipplescript-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Write a `recall`-using `@service` workflow plus a co-located portable
/// `whip.lock` (and its manifest copy) into `dir`, returning the workflow path.
fn write_locked_recall_project(bin: &str, dir: &Path) -> PathBuf {
    let workflow_path = dir.join("wf.whip");
    fs::write(
        &workflow_path,
        r#"
@service
workflow Recall

use memory

class WorkItem {
  title string
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule recall_before_work
  when WorkItem as item
  when worker is available
=> {
  recall project_memory for item as context

  after context succeeds {
    tell worker "{{ item.title }}"
  }
}
"#,
    )
    .expect("workflow writes");
    let memory_manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packages/memory.json");
    let manifest_copy = dir.join("memory.json");
    fs::copy(&memory_manifest, &manifest_copy).expect("copy manifest beside lock");
    run_text(
        bin,
        &[
            "package",
            "lock",
            "--output",
            dir.join("whip.lock").to_str().expect("utf-8 lock"),
            manifest_copy.to_str().expect("utf-8 manifest"),
        ],
    );
    workflow_path
}

#[test]
fn test_harness_reports_pass_fail_and_error_honestly() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
        r#"
workflow TriageDemo

output done Result

class Result {
  status string
}

rule complete_now
  when started
=> {
  complete done {
    status "ok"
  }
}

test "completes from start" {
  run until idle
  expect rule complete_now fired
  expect workflow completed
}

test "wrong rule expectation fails" {
  run until idle
  expect rule nonexistent_rule fired
}

test "unsupported stub outcome is invalid" {
  stub agent x retries
  run until idle
  expect workflow completed
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    // Failures present → non-zero exit.
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios array");
    let status_of = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                // Scenario ids are `<workflow>::<name>`; these single-workflow
                // fixtures look up by the trailing name.
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .and_then(|s| s.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
            .to_owned()
    };
    // The runner executes real scenarios and never false-passes. An assertion
    // that does not hold is `failed`; a scenario the harness cannot run
    // faithfully (here, an unsupported stub outcome) is `invalid` — never a pass.
    assert_eq!(status_of("completes from start"), "passed");
    assert_eq!(status_of("wrong rule expectation fails"), "failed");
    assert_eq!(status_of("unsupported stub outcome is invalid"), "invalid");
    // A scenario that cannot run surfaces its reason in `diagnostics`.
    let invalid = scenarios
        .iter()
        .find(|s| {
            s.get("id").and_then(Value::as_str)
                == Some("TriageDemo::unsupported stub outcome is invalid")
        })
        .expect("invalid scenario present");
    assert_eq!(
        invalid
            .get("diagnostics")
            .and_then(Value::as_array)
            .map(|d| d.len()),
        Some(1)
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_harness_evaluates_fact_projection_expects() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-proj");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
        r#"
@service
workflow Recorder

class Note {
  topic string
}

rule record_note
  when started
=> {
  record Note {
    topic "alpha"
  }
}

test "fact projection matches" {
  run until idle
  expect Note exists
  expect Note where topic == "alpha"
  expect Note count where topic == "alpha" is 1
  expect Note count where topic == "beta" is 0
}

test "wrong projection count fails" {
  run until idle
  expect Note count where topic == "alpha" is 5
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios array");
    let status_of = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                // Scenario ids are `<workflow>::<name>`; these single-workflow
                // fixtures look up by the trailing name.
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .and_then(|s| s.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
            .to_owned()
    };
    assert_eq!(status_of("fact projection matches"), "passed");
    assert_eq!(status_of("wrong projection count fails"), "failed");

    let _ = fs::remove_dir_all(&dir);
}

/// `given file <store> at <path> "<content>"` seeds a deterministic fixture into
/// the named `file store` and redirects its root, so a `read` runs through the
/// real worker against the fixture instead of the workflow's declared (here
/// non-existent) path. A read of an unseeded path fails honestly.
#[test]
fn test_harness_seeds_file_fixtures() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-file");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
        r#"
workflow FileReadTest

output result Result
failure error Stopped

class Result {
  status string
}

class Stopped {
  reason string
}

file store project_files {
  root "./does-not-exist"
}

rule pick
  when started
=> {
  read text from project_files at "note.md" as fileResult
  after fileResult succeeds as result {
    complete result {
      status "read-ok"
    }
  }
  after fileResult fails as err {
    fail error {
      reason "no file"
    }
  }
}

test "a seeded fixture is read and completes" {
  workflow FileReadTest
  given file project_files at "note.md" "seeded body"
  run until workflow completed
  expect workflow completed
}

test "an unseeded path fails the read" {
  workflow FileReadTest
  given file project_files at "other.md" "unrelated"
  run until workflow failed
  expect workflow failed
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios array");
    let status_of = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .and_then(|s| s.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
            .to_owned()
    };
    assert_eq!(
        status_of("a seeded fixture is read and completes"),
        "passed"
    );
    assert_eq!(status_of("an unseeded path fails the read"), "passed");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_harness_injects_coerce_output() {
    // `stub coerce <fn> returns { … }` controls the typed result a fixture coerce
    // returns, so a test can drive the value a workflow branches on and assert on
    // it. Two scenarios inject different verdicts and confirm each propagates.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-coerce");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
        r#"
workflow CoerceInject

output result Out

class Out {
  verdict string
}

class Decision {
  verdict "merge" | "revise" | "blocked"
}

coerce classify(text string) -> Decision {
  prompt """markdown
  Classify the text.
  """
}

rule run
  when started
=> {
  coerce classify("hello") as decision

  after decision succeeds {
    record Out {
      verdict decision.verdict
    }
    complete result {
      verdict decision.verdict
    }
  }
}

test "injects merge" {
  workflow CoerceInject
  stub coerce classify returns {
    verdict "merge"
  }
  run until idle
  expect workflow completed
  expect Out where verdict == "merge"
}

test "injects blocked" {
  workflow CoerceInject
  stub coerce classify returns {
    verdict "blocked"
  }
  run until idle
  expect Out where verdict == "blocked"
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios");
    let status_of = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                // Scenario ids are `<workflow>::<name>`; these single-workflow
                // fixtures look up by the trailing name.
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .and_then(|s| s.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
            .to_owned()
    };
    assert_eq!(status_of("injects merge"), "passed", "report: {report}");
    assert_eq!(status_of("injects blocked"), "passed");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_command_selection_patterns_and_exit_codes() {
    // `whip test` conforms to the CLI surface in `workflow-testing.md`: `-i`/`-x`
    // scenario selection over `<workflow>::<name>` ids with `*` globs, `--list`,
    // `--pass-if-no-tests`, and the spec exit codes (0 passed · 1 failures · 4
    // nothing selected).
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("test-select");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
        r#"
workflow Sel

output done Result

class Result {
  status string
}

rule done_now
  when started
=> {
  complete done {
    status "ok"
  }
}

test "alpha passes" {
  run until idle
  expect workflow completed
}

test "beta fails" {
  run until idle
  expect rule nope fired
}

test "gamma passes" {
  run until idle
  expect rule done_now fired
}
"#,
    )
    .expect("write workflow");
    let path = wf.to_str().expect("present");

    let run = |extra: &[&str]| {
        let mut args = vec!["test", path];
        args.extend_from_slice(extra);
        Command::new(bin)
            .args(&args)
            .output()
            .expect("whip test runs")
    };
    let code = |out: &std::process::Output| out.status.code().expect("exit code");

    // No selection: beta fails → overall failure, exit 1.
    let all = run(&[]);
    assert_eq!(code(&all), 1, "running all should exit 1 (beta fails)");

    // Select only the passing alpha by full id glob → exit 0.
    let alpha = run(&["-i", "Sel::alpha*"]);
    assert_eq!(code(&alpha), 0, "selecting only alpha should pass");

    // `::*passes` selects across workflows by test name → alpha + gamma, exit 0.
    let passes = run(&["--json", "-i", "::*passes"]);
    assert_eq!(code(&passes), 0);
    let report: Value = serde_json::from_slice(&passes.stdout).expect("json report");
    assert_eq!(
        report.pointer("/summary/selected").and_then(Value::as_u64),
        Some(2),
        "::*passes should select two scenarios: {report}"
    );

    // Excluding the only failing test leaves two passing → exit 0.
    let no_beta = run(&["-x", "Sel::beta fails"]);
    assert_eq!(code(&no_beta), 0, "excluding beta should pass");

    // Nothing matches → exit 4 …
    let none = run(&["-i", "Sel::nonexistent"]);
    assert_eq!(code(&none), 4, "no selection should exit 4");
    // … unless --pass-if-no-tests downgrades it to success.
    let none_ok = run(&["-i", "Sel::nonexistent", "--pass-if-no-tests"]);
    assert_eq!(code(&none_ok), 0, "--pass-if-no-tests should exit 0");

    // --list enumerates without running: all three ids, exit 0.
    let list = run(&["--list"]);
    assert_eq!(code(&list), 0);
    let listed = String::from_utf8(list.stdout).expect("utf-8");
    assert_eq!(
        listed.lines().count(),
        3,
        "--list should print all three ids: {listed:?}"
    );
    assert!(listed.contains("Sel::beta fails"));

    // --list honors selection.
    let list_alpha = run(&["--list", "-i", "::alpha*"]);
    let listed_alpha = String::from_utf8(list_alpha.stdout).expect("utf-8");
    assert_eq!(listed_alpha.trim(), "Sel::alpha passes");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_command_aggregates_multiple_sources() {
    // `whip test a.whip b.whip` compiles each source and aggregates their
    // scenarios into one report; selection and exit codes span all sources, and
    // each scenario runs against its own program text.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("test-multi");
    let one = dir.join("one.whip");
    let two = dir.join("two.whip");
    fs::write(
        &one,
        r#"
workflow One

output done Result

class Result {
  status string
}

rule done_now
  when started
=> {
  complete done {
    status "ok"
  }
}

test "one passes" {
  run until idle
  expect workflow completed
}
"#,
    )
    .expect("write one");
    fs::write(
        &two,
        r#"
workflow Two

output done Result

class Result {
  status string
}

rule done_now
  when started
=> {
  complete done {
    status "ok"
  }
}

test "two passes" {
  run until idle
  expect workflow completed
}

test "two fails" {
  run until idle
  expect rule nope fired
}
"#,
    )
    .expect("write two");
    let one_path = one.to_str().expect("present");
    let two_path = two.to_str().expect("present");

    // Both sources, JSON: three scenarios aggregated, overall failed (exit 1).
    let both = Command::new(bin)
        .args(["--json", "test", one_path, two_path])
        .output()
        .expect("whip test runs");
    assert_eq!(both.status.code(), Some(1), "two fails → exit 1");
    let report: Value = serde_json::from_slice(&both.stdout).expect("json report");
    assert_eq!(
        report.pointer("/summary/selected").and_then(Value::as_u64),
        Some(3),
        "all three scenarios across both files: {report}"
    );
    let ids: Vec<&str> = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios")
        .iter()
        .filter_map(|s| s.get("id").and_then(Value::as_str))
        .collect();
    assert!(ids.contains(&"One::one passes"), "ids: {ids:?}");
    assert!(ids.contains(&"Two::two fails"), "ids: {ids:?}");

    // Selection spans sources: include only the passing scenarios across files.
    let passes = Command::new(bin)
        .args(["test", one_path, two_path, "-i", "::*passes"])
        .output()
        .expect("whip test runs");
    assert_eq!(
        passes.status.code(),
        Some(0),
        "only passing selected → exit 0"
    );

    // A compile error in any source is setup-invalid (exit 2), before running.
    let bad = dir.join("bad.whip");
    fs::write(&bad, "workflow Bad\nrule oops =>\n").expect("write bad");
    let with_bad = Command::new(bin)
        .args(["test", one_path, bad.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    assert_eq!(with_bad.status.code(), Some(2), "compile error → exit 2");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_command_discovers_whip_files_in_a_directory() {
    // A directory positional is discovered recursively into its `.whip` files
    // (skipping hidden dirs), and their scenarios are aggregated like explicit
    // files. Selection and exit codes span the discovered set.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("test-dir");
    let nested = dir.join("nested");
    fs::create_dir_all(&nested).expect("mkdir nested");
    let body = |workflow: &str, test: &str, expect: &str| {
        format!(
            r#"
workflow {workflow}

output done Result

class Result {{
  status string
}}

rule done_now
  when started
=> {{
  complete done {{
    status "ok"
  }}
}}

test "{test}" {{
  run until idle
  {expect}
}}
"#
        )
    };
    fs::write(
        dir.join("top.whip"),
        body("Top", "top passes", "expect workflow completed"),
    )
    .expect("write top");
    fs::write(
        nested.join("deep.whip"),
        body("Deep", "deep passes", "expect rule done_now fired"),
    )
    .expect("write deep");
    // A hidden directory must be skipped by discovery.
    let hidden = dir.join(".hidden");
    fs::create_dir_all(&hidden).expect("mkdir hidden");
    fs::write(
        hidden.join("skip.whip"),
        body("Skip", "should not run", "expect rule nope fired"),
    )
    .expect("write hidden");

    let out = Command::new(bin)
        .args(["--json", "test", dir.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    assert_eq!(out.status.code(), Some(0), "discovered scenarios all pass");
    let report: Value = serde_json::from_slice(&out.stdout).expect("json report");
    let ids: Vec<&str> = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios")
        .iter()
        .filter_map(|s| s.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(ids.len(), 2, "two discovered (hidden skipped): {ids:?}");
    assert!(ids.contains(&"Top::top passes"), "ids: {ids:?}");
    assert!(ids.contains(&"Deep::deep passes"), "ids: {ids:?}");
    assert!(
        !ids.iter().any(|id| id.starts_with("Skip::")),
        "hidden dir must be skipped: {ids:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_replay_verifies_event_log_reprojects_identically() {
    // `whip test replay <instance>` replays the recorded event log into a
    // throwaway copy of the store and confirms the reconstructed projection is
    // byte-identical to the live-built one (replay equality). The user's store is
    // not mutated. An unknown instance is a setup error (exit 2).
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("replay");
    let store = dir.join("store.sqlite");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
        r#"
workflow ReplayMe

output done Result

class Result {
  status string
}

rule done_now
  when started
=> {
  complete done {
    status "ok"
  }
}
"#,
    )
    .expect("write workflow");
    let store_str = store.to_str().expect("present");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_str,
            "--json",
            "dev",
            wf.to_str().expect("present"),
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

    let replay = Command::new(bin)
        .args(["--store", store_str, "--json", "test", "replay", &instance])
        .output()
        .expect("whip test replay runs");
    assert_eq!(
        replay.status.code(),
        Some(0),
        "replay should be equal; stderr: {}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let report: Value = serde_json::from_slice(&replay.stdout).expect("replay report JSON");
    assert_eq!(
        report.get("replay").and_then(Value::as_str),
        Some("equal"),
        "report: {report}"
    );

    // The original store is untouched — its instance still resolves.
    let status = run_json(bin, &["--store", store_str, "--json", "status", &instance]);
    assert!(status.get("instance").is_some(), "store intact: {status}");

    // Unknown instance → setup error (exit 2).
    let missing = Command::new(bin)
        .args(["--store", store_str, "test", "replay", "no-such-instance"])
        .output()
        .expect("whip test replay runs");
    assert_eq!(missing.status.code(), Some(2), "unknown instance is exit 2");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_harness_given_tracker_seeds_builtin_tracker_issue() {
    // `given tracker <name> issue { … }` seeds an existing issue into the builtin
    // tracker (isolated per scenario). The workflow's queue projection surfaces it
    // as a `tracker.issue.ready` fact — going through the real projection path — so a
    // `<queue> has ready issue` rule fires on it.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-tracker");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
        r#"
workflow TrackerSeed

tracker backlog {
  provider builtin
}

output done Result

class Result {
  title string
}

rule pick
  when {
    backlog has ready issue as item
  }
=> {
  complete done {
    title item.title
  }
}

test "seeded issue is picked up" {
  given tracker backlog issue {
    title "Existing issue"
  }
  run until idle
  expect tracker.issue.ready where title == "Existing issue"
  expect rule pick fired
  expect workflow completed
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenario = report
        .get("scenarios")
        .and_then(Value::as_array)
        .and_then(|s| s.first())
        .expect("one scenario");
    assert_eq!(
        scenario.get("status").and_then(Value::as_str),
        Some("passed"),
        "report: {report}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_harness_given_clock_controls_deadline_firing() {
    // `given clock at "…"` injects a virtual evaluation clock so a deadline test
    // is deterministic and instant — no wall-clock sleep. The workflow queues a
    // `timer until job.dueAt` deadline (2026-06-11). One scenario advances the
    // clock past it (deadline fires → workflow fails), the other holds the clock
    // before it (deadline stays pending → no failure). Same program, opposite
    // outcomes driven solely by the injected clock.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-clock");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
        r#"
workflow ClockDeadline

failure error Expired

class Job {
  id string
  dueAt time
  status "waiting"
}

class Expired {
  id string
  reason string
}

table jobs as Job [
  {
    id "J-1"
    dueAt "2026-06-11T18:00:00Z"
    status "waiting"
  }
]

rule begin
  when Job as job where job.status == "waiting"
=> {
  timer until job.dueAt as deadline

  after deadline succeeds {
    done job
    record Expired {
      id job.id
      reason "deadline reached"
    }
    fail error {
      id job.id
      reason "deadline reached"
    }
  }
}

test "clock past the deadline fires it" {
  workflow ClockDeadline
  given clock at "2030-01-01T00:00:00Z"
  run until idle
  expect rule begin fired
  expect effect timer.wait completed
  expect workflow failed
  expect Expired where reason == "deadline reached"
}

test "clock before the deadline holds it" {
  workflow ClockDeadline
  given clock at "2020-01-01T00:00:00Z"
  run until idle
  expect rule begin fired
  expect Expired count where reason == "deadline reached" is 0
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios");
    let status_of = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                // Scenario ids are `<workflow>::<name>`; these single-workflow
                // fixtures look up by the trailing name.
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .and_then(|s| s.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
            .to_owned()
    };
    assert_eq!(
        status_of("clock past the deadline fires it"),
        "passed",
        "report: {report}"
    );
    assert_eq!(
        status_of("clock before the deadline holds it"),
        "passed",
        "report: {report}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn interval_clock_source_fires_occurrences_at_runtime() {
    // An `every <duration>` clock source admits a durable signal occurrence at the
    // worker boundary when its interval has elapsed (spec/std-time.md). A reacting
    // rule then fires. With the default `coalesce` policy a long-idle source admits
    // a single representative occurrence rather than one fact per elapsed interval.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("clock-source");
    let wf = dir.join("clock_interval.whip");
    fs::write(
        &wf,
        r#"@service
workflow ClockInterval

signal heartbeat.tick {
  scheduled_at time
  observed_at time
  occurrence_id string
  missed_count int
}

class Beat {
  id string
}

source clock as beat {
  every 1h
  missed coalesce

  observe as tick
  emit heartbeat.tick {
    scheduled_at tick.scheduled_at
    observed_at tick.observed_at
    occurrence_id tick.occurrence_id
    missed_count tick.missed_count
  }
}

rule on_beat
  when heartbeat.tick as tick
=> {
  record Beat {
    id tick.occurrence_id
  }
}

test "interval clock fires when due" {
  workflow ClockInterval
  given clock at "2030-01-01T00:00:00Z"
  run until idle
  expect rule on_beat fired
  expect Beat count where id != "" is 1
}

test "interval clock holds before the first tick" {
  workflow ClockInterval
  given clock at "1970-01-01T00:00:00Z"
  run until idle
  expect Beat count where id != "" is 0
}
"#,
    )
    .expect("write clock workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios");
    let status_of = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .and_then(|s| s.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
            .to_owned()
    };
    assert_eq!(
        status_of("interval clock fires when due"),
        "passed",
        "report: {report}"
    );
    assert_eq!(
        status_of("interval clock holds before the first tick"),
        "passed",
        "report: {report}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn calendar_clock_source_fires_occurrences_at_runtime() {
    // An `every <calendar> at <time>` clock source admits tz-aware, DST-correct
    // occurrences at the worker boundary (spec/std-time.md), the calendar analogue
    // of the interval source. A far-future `given clock` means many daily 09:00
    // occurrences have elapsed; `coalesce` admits one representative occurrence.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("clock-calendar");
    let wf = dir.join("clock_calendar.whip");
    fs::write(
        &wf,
        r#"@service
workflow ClockCalendar

signal triage.tick {
  scheduled_at time
  observed_at time
  occurrence_id string
  missed_count int
}

class Beat {
  id string
}

source clock as daily_triage {
  every day at 09:00
  timezone "America/New_York"
  missed coalesce

  observe as tick
  emit triage.tick {
    scheduled_at tick.scheduled_at
    observed_at tick.observed_at
    occurrence_id tick.occurrence_id
    missed_count tick.missed_count
  }
}

rule on_tick
  when triage.tick as tick
=> {
  record Beat {
    id tick.occurrence_id
  }
}

test "calendar clock fires when due" {
  workflow ClockCalendar
  given clock at "2030-06-03T14:00:00Z"
  run until idle
  expect rule on_tick fired
  expect Beat count where id != "" is 1
}

test "calendar clock holds before the first occurrence" {
  workflow ClockCalendar
  given clock at "1970-01-01T00:00:00Z"
  run until idle
  expect Beat count where id != "" is 0
}
"#,
    )
    .expect("write calendar clock workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios");
    let status_of = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .and_then(|s| s.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
            .to_owned()
    };
    assert_eq!(
        status_of("calendar clock fires when due"),
        "passed",
        "report: {report}"
    );
    assert_eq!(
        status_of("calendar clock holds before the first occurrence"),
        "passed",
        "report: {report}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_flags_unused_coerce_functions() {
    // `whip lint` reports a coerce declared but never called (dead code), and does
    // not flag a coerce that is called. A coerce is only callable within the
    // program, so this has no false positives.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint");
    let wf = dir.join("lint.whip");
    fs::write(
        &wf,
        r#"workflow LintDemo

output result R

class R {
  ok bool
}

class Ticket {
  title string
}

coerce assessUsed(title string) -> R {
  client "X"
}

coerce assessDead(title string) -> R {
  client "X"
}

rule run
  when Ticket as t
=> {
  coerce assessUsed(t.title) as a
  after a succeeds as outcome {
    complete result {
      ok outcome.ok
    }
  }
}
"#,
    )
    .expect("write lint workflow");

    let output = Command::new(bin)
        .args(["--json", "lint", wf.to_str().expect("present")])
        .output()
        .expect("whip lint runs");
    assert!(output.status.success(), "lint should exit 0 on warnings");
    let report: Value = serde_json::from_slice(&output.stdout).expect("lint report JSON");
    let findings = report
        .get("findings")
        .and_then(Value::as_array)
        .expect("findings");
    let unused: Vec<&str> = findings
        .iter()
        .filter(|f| f.get("code").and_then(Value::as_str) == Some("lint.unused_coerce"))
        .filter_map(|f| f.get("message").and_then(Value::as_str))
        .collect();
    assert_eq!(unused.len(), 1, "exactly one unused coerce: {report}");
    assert!(
        unused[0].contains("assessDead"),
        "the dead coerce is flagged: {report}"
    );
    assert!(
        !unused[0].contains("assessUsed"),
        "the called coerce is not flagged: {report}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_flags_unused_coerce_result() {
    // A coerce that IS called but whose result binding is never used is flagged
    // (`lint.coerce_result_unused`); the same coerce with its result handled by an
    // `after <binding>` block is not.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-coerce-result");

    let program = |body: &str| {
        format!(
            r#"workflow CoerceResult

output result R

class R {{
  ok bool
}}

class Ticket {{
  title string
}}

coerce assess(title string) -> R {{
  client "X"
}}

rule run
  when Ticket as t
=> {{
{body}
}}
"#
        )
    };

    let unused = dir.join("unused.whip");
    fs::write(
        &unused,
        program("  coerce assess(t.title) as verdict\n  complete result {\n    ok true\n  }"),
    )
    .expect("write workflow");
    let used = dir.join("used.whip");
    fs::write(
        &used,
        program(
            "  coerce assess(t.title) as verdict\n  after verdict succeeds as v {\n    complete result {\n      ok v.ok\n    }\n  }",
        ),
    )
    .expect("write workflow");

    let codes = |path: &std::path::Path| -> Vec<String> {
        let output = Command::new(bin)
            .args(["--json", "lint", path.to_str().expect("present")])
            .output()
            .expect("lint runs");
        let report: Value = serde_json::from_slice(&output.stdout).expect("lint JSON");
        report
            .get("findings")
            .and_then(Value::as_array)
            .expect("findings")
            .iter()
            .filter_map(|f| f.get("code").and_then(Value::as_str).map(str::to_owned))
            .collect()
    };

    assert!(
        codes(&unused).contains(&"lint.coerce_result_unused".to_owned()),
        "an unused coerce result must be flagged"
    );
    assert!(
        !codes(&used).contains(&"lint.coerce_result_unused".to_owned()),
        "a handled coerce result must not be flagged"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_findings_carry_declaration_spans() {
    // Every lint finding resolves to the source span of the declaration it concerns,
    // so editors and the CLI can point at it. Here the dead coerce's `range` must
    // start on the line where `coerce assessDead` is declared.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint");
    let wf = dir.join("lint.whip");
    let source = r#"workflow LintDemo

output result R

class R {
  ok bool
}

class Ticket {
  title string
}

coerce assessDead(title string) -> R {
  client "X"
}

rule run
  when Ticket as t
=> {
  complete result {
    ok true
  }
}
"#;
    fs::write(&wf, source).expect("write lint workflow");
    // The declaration line of `coerce assessDead` (0-based for LSP-style ranges).
    let dead_line = source
        .lines()
        .position(|line| line.starts_with("coerce assessDead"))
        .expect("dead coerce line") as u64;

    let output = Command::new(bin)
        .args(["--json", "lint", wf.to_str().expect("present")])
        .output()
        .expect("whip lint runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("lint report JSON");
    let finding = report
        .get("findings")
        .and_then(Value::as_array)
        .expect("findings")
        .iter()
        .find(|f| f.get("code").and_then(Value::as_str) == Some("lint.unused_coerce"))
        .expect("the unused coerce finding");
    let start_line = finding
        .get("range")
        .and_then(|range| range.get("start"))
        .and_then(|start| start.get("line"))
        .and_then(Value::as_u64)
        .expect("finding carries a resolved range");
    assert_eq!(
        start_line, dead_line,
        "lint range should point at the declaration line: {report}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// A workflow with exactly one lint finding (`lint.unused_class` on `Orphan`).
fn lint_actions_workflow(dir: &std::path::Path) -> std::path::PathBuf {
    let wf = dir.join("lint.whip");
    fs::write(
        &wf,
        r#"workflow LintActions

output result R

class R {
  ok bool
}

class Ticket {
  title string
}

class Orphan {
  note string
}

rule run
  when Ticket as t
=> {
  complete result {
    ok true
  }
}
"#,
    )
    .expect("write lint workflow");
    wf
}

#[test]
fn lint_flags_deep_after_nesting() {
    // A rule nesting `after` blocks ≥4 levels deep is flagged (suggest a `flow`); a
    // shallow chain is not.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-deep");
    // n levels of nested `after`, each behind its own coerce.
    let nest = |levels: usize| -> String {
        let coerces: String = (1..=levels)
            .map(|i| format!("coerce f{i}(id string) -> R {{ client \"X\" }}\n"))
            .collect();
        let mut body = String::from("  complete result {\n    ok true\n  }\n");
        for i in (1..=levels).rev() {
            body = format!(
                "  coerce f{i}(t.id) as a{i}\n  after a{i} succeeds as r{i} {{\n{body}  }}\n"
            );
        }
        format!(
            "workflow Deep\n\noutput result R\nclass R {{ ok bool }}\nclass Ticket {{ id string }}\n\n{coerces}\nrule deep\n  when Ticket as t\n=> {{\n{body}}}\n"
        )
    };

    let codes = |levels: usize| -> Vec<String> {
        let wf = dir.join("deep.whip");
        fs::write(&wf, nest(levels)).expect("write workflow");
        let output = Command::new(bin)
            .args(["--json", "lint", wf.to_str().expect("present")])
            .output()
            .expect("lint runs");
        let report: Value = serde_json::from_slice(&output.stdout).expect("lint JSON");
        report
            .get("findings")
            .and_then(Value::as_array)
            .expect("findings")
            .iter()
            .filter_map(|f| f.get("code").and_then(Value::as_str).map(str::to_owned))
            .collect()
    };

    assert!(
        codes(4).contains(&"lint.deep_after_nesting".to_owned()),
        "4-deep nesting must be flagged"
    );
    assert!(
        !codes(2).contains(&"lint.deep_after_nesting".to_owned()),
        "shallow nesting must not be flagged"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_discovers_directory_sources_and_aggregates() {
    // `whip lint <dir>` discovers `.whip` files recursively and aggregates findings;
    // the multi-source JSON uses a `reports` array, and a denied finding in any file
    // fails the run.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-dir");
    let program = |orphan: &str| {
        format!(
            r#"workflow W

class Used {{
  id string
}}

class {orphan} {{
  x string
}}

rule run
  when Used as u
=> {{
  done u
}}
"#
        )
    };
    fs::write(dir.join("a.whip"), program("OrphanA")).expect("write a");
    fs::write(dir.join("b.whip"), program("OrphanB")).expect("write b");

    // Directory discovery → a `reports` array with one entry per file.
    let output = Command::new(bin)
        .args(["--json", "lint", dir.to_str().expect("present")])
        .output()
        .expect("lint runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("lint JSON");
    let reports = report
        .get("reports")
        .and_then(Value::as_array)
        .expect("reports array for multiple sources");
    assert_eq!(reports.len(), 2, "one report per discovered file: {report}");

    // A denied finding in any discovered file fails the whole run.
    let denied = Command::new(bin)
        .args([
            "lint",
            "--deny",
            "lint.unused_class",
            dir.to_str().expect("present"),
        ])
        .output()
        .expect("lint runs");
    assert!(
        !denied.status.success(),
        "a denied finding in any file fails the run"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_flags_broad_file_grant() {
    // A file-store grant that matches everything under the root (`**`) is flagged;
    // a scoped glob (`docs/**`) is not.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-grant");

    let program = |glob: &str| {
        format!(
            r#"workflow FileRead

output result Result

class Result {{
  status string
}}

file store project_files {{
  root "./data"
  allow read [{glob}]
}}

rule pick
  when started
=> {{
  read text from project_files at "note.md" as fileResult
  after fileResult succeeds as result {{
    complete result {{
      status "read-ok"
    }}
  }}
}}
"#
        )
    };

    let codes = |glob: &str| -> Vec<String> {
        let wf = dir.join("store.whip");
        fs::write(&wf, program(glob)).expect("write workflow");
        let output = Command::new(bin)
            .args(["--json", "lint", wf.to_str().expect("present")])
            .output()
            .expect("lint runs");
        let report: Value = serde_json::from_slice(&output.stdout).expect("lint JSON");
        report
            .get("findings")
            .and_then(Value::as_array)
            .expect("findings")
            .iter()
            .filter_map(|f| f.get("code").and_then(Value::as_str).map(str::to_owned))
            .collect()
    };

    assert!(
        codes("\"**\"").contains(&"lint.broad_file_grant".to_owned()),
        "a `**` grant must be flagged"
    );
    assert!(
        !codes("\"docs/**\"").contains(&"lint.broad_file_grant".to_owned()),
        "a scoped grant must not be flagged"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_rule_selection_restricts_to_named_rules() {
    // `--rule <id>` runs only the named rule(s); other findings are not emitted.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-rule");
    let wf = dir.join("multi.whip");
    // Two findings: an unused class (Orphan) and an unused coerce result (verdict).
    fs::write(
        &wf,
        r#"workflow Multi

output result R

class R {
  ok bool
}

class Ticket {
  title string
}

class Orphan {
  x string
}

coerce assess(title string) -> R {
  client "X"
}

rule run
  when Ticket as t
=> {
  coerce assess(t.title) as verdict
  complete result {
    ok true
  }
}
"#,
    )
    .expect("write workflow");

    let codes = |args: &[&str]| -> Vec<String> {
        let mut full = vec!["--json", "lint"];
        full.extend_from_slice(args);
        full.push(wf.to_str().expect("present"));
        let output = Command::new(bin).args(&full).output().expect("lint runs");
        let report: Value = serde_json::from_slice(&output.stdout).expect("lint JSON");
        let mut found: Vec<String> = report
            .get("findings")
            .and_then(Value::as_array)
            .expect("findings")
            .iter()
            .filter_map(|f| f.get("code").and_then(Value::as_str).map(str::to_owned))
            .collect();
        found.sort();
        found.dedup();
        found
    };

    assert!(
        codes(&[]).len() >= 2,
        "without --rule both findings are emitted"
    );
    assert_eq!(
        codes(&["--rule", "lint.unused_class"]),
        vec!["lint.unused_class".to_owned()],
        "--rule restricts to the named rule only"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_deny_action_exits_nonzero() {
    // `--deny <id>` reports the finding and fails the run; the default `warn` does not.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-deny");
    let wf = lint_actions_workflow(&dir);

    let warn = Command::new(bin)
        .args(["lint", wf.to_str().expect("present")])
        .output()
        .expect("lint runs");
    assert!(
        warn.status.success(),
        "default action does not fail the run"
    );

    let deny = Command::new(bin)
        .args([
            "lint",
            "--deny",
            "lint.unused_class",
            wf.to_str().expect("present"),
        ])
        .output()
        .expect("lint runs");
    assert!(!deny.status.success(), "a denied finding fails the run");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_allow_action_suppresses_finding() {
    // `--allow <id>` suppresses the finding entirely (not emitted, exit 0).
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-allow");
    let wf = lint_actions_workflow(&dir);

    let output = Command::new(bin)
        .args([
            "--json",
            "lint",
            "--allow",
            "lint.unused_class",
            wf.to_str().expect("present"),
        ])
        .output()
        .expect("lint runs");
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("lint JSON");
    let findings = report
        .get("findings")
        .and_then(Value::as_array)
        .expect("findings");
    assert!(
        findings.is_empty(),
        "allowed finding must be suppressed: {report}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_config_file_applies_and_cli_overrides() {
    // A project `whip.lint.json` can deny a rule; a CLI `--allow` overrides it.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-config");
    let wf = lint_actions_workflow(&dir);
    fs::write(
        dir.join("whip.lint.json"),
        r#"{ "schema": "whipplescript.lint_config.v0", "rules": { "lint.unused_class": "deny" } }"#,
    )
    .expect("write lint config");

    let config_deny = Command::new(bin)
        .args(["lint", wf.to_str().expect("present")])
        .output()
        .expect("lint runs");
    assert!(!config_deny.status.success(), "config deny fails the run");

    let cli_override = Command::new(bin)
        .args([
            "lint",
            "--allow",
            "lint.unused_class",
            wf.to_str().expect("present"),
        ])
        .output()
        .expect("lint runs");
    assert!(
        cli_override.status.success(),
        "CLI --allow overrides config deny"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_invalid_config_is_internal_error() {
    // An invalid `whip.lint.json` is a `lint.internal` infrastructure error (exit
    // nonzero), distinct from a lint finding.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-badconfig");
    let wf = lint_actions_workflow(&dir);
    fs::write(
        dir.join("whip.lint.json"),
        r#"{ "schema": "whipplescript.lint_config.v0", "rules": { "lint.unused_class": "nope" } }"#,
    )
    .expect("write lint config");

    let output = Command::new(bin)
        .args(["lint", wf.to_str().expect("present")])
        .output()
        .expect("lint runs");
    assert!(!output.status.success(), "invalid config fails the run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("lint.internal"), "stderr:\n{stderr}");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lsp_publishes_lint_findings_as_diagnostics() {
    // On a program that compiles, `whip lsp` surfaces lint findings as diagnostics
    // tagged `whip lint` (distinct from the `whip` correctness diagnostics via that
    // source + the `lint.*` code), at the finding's own severity (a warning → LSP
    // severity 2), each pointing at the offending declaration.
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    // A valid program whose `Orphan` class is never referenced.
    let text = "workflow Demo\\nclass Ticket {\\n  id string\\n}\\nclass Orphan {\\n  x string\\n}\\nrule r\\n  when Ticket as t\\n=> {\\n  done t\\n}\\n";
    let mut input = String::new();
    input += &frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///lint.whip","text":"{text}"}}}}}}"#
    ));
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains(r#""source":"whip lint""#),
        "lint findings should publish as `whip lint` diagnostics: {stdout}"
    );
    assert!(
        stdout.contains(r#""code":"lint.unused_class""#),
        "the orphan class lint finding should be reported: {stdout}"
    );
    // The lint diagnostic carries the finding's own severity (warning → LSP 2).
    // serde_json sorts keys alphabetically, so `severity` precedes `source`.
    assert!(
        stdout.contains(r#""severity":2,"source":"whip lint""#),
        "lint diagnostics should use the finding's severity (warning = LSP 2): {stdout}"
    );
}

#[test]
fn lint_flags_unused_coordination_resources() {
    // `whip lint` reports a lease/ledger/counter declared but never referenced.
    // Like coerce, coordination resources are only usable in-program, so an
    // unreferenced one is dead (no false positives). A referenced one is not flagged.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-res");
    let wf = dir.join("lint_res.whip");
    fs::write(
        &wf,
        r#"@service
workflow LintRes

class Env {
  name string
}

class Decision {
  area string
}

class Customer {
  id string
}

class Ticket {
  env string
}

table tickets as Ticket [
  { env "prod" }
]

lease used_slot { key Env  slots 1  ttl 10m }
lease dead_slot { key Env  slots 1  ttl 10m }
ledger dead_log { entry Decision  partition by area  retain 90d }
counter dead_budget { key Customer  cap 1000  reset daily }
tracker used_q { provider builtin }
tracker dead_q { provider builtin }

rule run
  when Ticket as t
=> {
  acquire used_slot for t.env until ttl as slot

  after slot held {
    done t
  }

  after slot contended {
    done t
  }
}

rule drain
  when used_q has ready issue as item
=> {
  claim item as claimed

  after claimed succeeds as c {
    finish item
  }
}
"#,
    )
    .expect("write lint-res workflow");

    let output = Command::new(bin)
        .args(["--json", "lint", wf.to_str().expect("present")])
        .output()
        .expect("whip lint runs");
    assert!(output.status.success(), "lint should exit 0 on warnings");
    let report: Value = serde_json::from_slice(&output.stdout).expect("lint report JSON");
    let findings = report
        .get("findings")
        .and_then(Value::as_array)
        .expect("findings");
    let codes_and_messages: Vec<(&str, &str)> = findings
        .iter()
        .filter_map(|f| {
            Some((
                f.get("code").and_then(Value::as_str)?,
                f.get("message").and_then(Value::as_str)?,
            ))
        })
        .collect();
    for (code, name) in [
        ("lint.unused_lease", "dead_slot"),
        ("lint.unused_ledger", "dead_log"),
        ("lint.unused_counter", "dead_budget"),
        ("lint.unused_tracker", "dead_q"),
    ] {
        assert!(
            codes_and_messages
                .iter()
                .any(|(c, m)| *c == code && m.contains(name)),
            "expected {code} for {name}: {report}"
        );
    }
    // `used_q` is referenced only by a `when ... has ready issue` clause, not the
    // body — so the when-clause scan must keep it from being flagged.
    assert!(
        !codes_and_messages.iter().any(|(_, m)| m.contains("used_q")),
        "a queue referenced via `when` must not be flagged: {report}"
    );
    assert!(
        !codes_and_messages
            .iter()
            .any(|(_, m)| m.contains("used_slot")),
        "the acquired lease must not be flagged: {report}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_flags_noop_rule_with_empty_body() {
    // `whip lint` flags a rule whose body is empty (it fires but does nothing — a
    // forgotten body) and does not flag a rule that produces output.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-noop");
    let wf = dir.join("lint_noop.whip");
    fs::write(
        &wf,
        r#"@service
workflow LintNoop

class Ticket {
  id string
}

class Seen {
  id string
}

table tickets as Ticket [
  { id "T1" }
]

rule empty
  when Ticket as t
=> {
}

rule records
  when Seen as s
=> {
  done s
}
"#,
    )
    .expect("write lint-noop workflow");

    let output = Command::new(bin)
        .args(["--json", "lint", wf.to_str().expect("present")])
        .output()
        .expect("whip lint runs");
    assert!(output.status.success(), "lint should exit 0 on warnings");
    let report: Value = serde_json::from_slice(&output.stdout).expect("lint report JSON");
    let noops: Vec<&str> = report
        .get("findings")
        .and_then(Value::as_array)
        .expect("findings")
        .iter()
        .filter(|f| f.get("code").and_then(Value::as_str) == Some("lint.noop_rule"))
        .filter_map(|f| f.get("message").and_then(Value::as_str))
        .collect();
    assert_eq!(noops.len(), 1, "exactly one no-op rule: {report}");
    assert!(
        noops[0].contains("empty"),
        "the empty rule is flagged: {report}"
    );
    assert!(
        !noops[0].contains("records"),
        "a producing rule is not flagged: {report}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lint_flags_unused_class_and_enum_types() {
    // `whip lint` flags a class/enum declared but referenced nowhere, and does not
    // flag a referenced one. Types are program-internal, so an unreferenced one is
    // dead (no false positives); synthetic lowering-generated types (never in the
    // source) are excluded.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("lint-types");
    let wf = dir.join("lint_types.whip");
    fs::write(
        &wf,
        r#"workflow LintTypes

output result R

class R {
  ok bool
}

class DeadClass {
  x string
}

enum DeadEnum {
  A
  B
}

enum UsedEnum {
  X
  Y
}

class Ticket {
  kind UsedEnum
}

table tk as Ticket [
  { kind X }
]

rule r
  when Ticket as t
=> {
  complete result {
    ok true
  }
}
"#,
    )
    .expect("write lint-types workflow");

    let output = Command::new(bin)
        .args(["--json", "lint", wf.to_str().expect("present")])
        .output()
        .expect("whip lint runs");
    assert!(output.status.success(), "lint should exit 0 on warnings");
    let report: Value = serde_json::from_slice(&output.stdout).expect("lint report JSON");
    let messages: Vec<&str> = report
        .get("findings")
        .and_then(Value::as_array)
        .expect("findings")
        .iter()
        .filter(|f| {
            matches!(
                f.get("code").and_then(Value::as_str),
                Some("lint.unused_class") | Some("lint.unused_enum")
            )
        })
        .filter_map(|f| f.get("message").and_then(Value::as_str))
        .collect();
    assert!(
        messages.iter().any(|m| m.contains("DeadClass")),
        "dead class flagged: {report}"
    );
    assert!(
        messages.iter().any(|m| m.contains("DeadEnum")),
        "dead enum flagged: {report}"
    );
    for referenced in ["UsedEnum", "Ticket", "`R`"] {
        assert!(
            !messages.iter().any(|m| m.contains(referenced)),
            "referenced type {referenced} must not be flagged: {report}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lsp_publishes_diagnostics_on_did_open() {
    // The language server speaks LSP over stdio: respond to `initialize`, and on
    // `textDocument/didOpen` compile the document and publish its diagnostics. Drive
    // it with framed JSON-RPC and assert it answers initialize and reports the
    // compile error (live error squiggles reuse the `whip check` compiler).
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let mut input = String::new();
    input += &frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input += &frame(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#);
    input += &frame(
        r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///t.whip","text":"workflow Demo\nclass C {\n  x\n}\n"}}}"#,
    );
    input += &frame(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown","params":{}}"#);
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("\"id\":1"),
        "server answered initialize: {stdout}"
    );
    assert!(
        stdout.contains("textDocument/publishDiagnostics"),
        "server published diagnostics: {stdout}"
    );
    assert!(
        stdout.contains("\"severity\":1"),
        "server reported an error diagnostic: {stdout}"
    );
}

#[test]
fn lsp_returns_document_symbols() {
    // After opening a document, `textDocument/documentSymbol` returns the top-level
    // declarations (the editor outline), reusing the parser's `document_symbols`.
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let text = "workflow Demo\\nclass Ticket {\\n  id string\\n}\\nrule handle\\n  when Ticket as t\\n=> {\\n  done t\\n}\\n";
    let mut input = String::new();
    input += &frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///s.whip","text":"{text}"}}}}}}"#
    ));
    input += &frame(
        r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/documentSymbol","params":{"textDocument":{"uri":"file:///s.whip"}}}"#,
    );
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The documentSymbol response (id 2) lists the workflow, class, and rule.
    for symbol in [
        "\"name\":\"Demo\"",
        "\"name\":\"Ticket\"",
        "\"name\":\"handle\"",
    ] {
        assert!(
            stdout.contains(symbol),
            "documentSymbol should list {symbol}: {stdout}"
        );
    }
}

#[test]
fn lsp_go_to_definition_resolves_top_level_name() {
    // `textDocument/definition` on a reference to a top-level declaration (here the
    // `when Ticket` reference on line 5) resolves to that declaration's range (the
    // `class Ticket` on lines 1-3). Top-level names are program-unique, so the
    // name match is the definition.
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let text = "workflow Demo\\nclass Ticket {\\n  id string\\n}\\nrule r\\n  when Ticket as t\\n=> {\\n  done t\\n}\\n";
    let mut input = String::new();
    input += &frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///d.whip","text":"{text}"}}}}}}"#
    ));
    input += &frame(
        r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/definition","params":{"textDocument":{"uri":"file:///d.whip"},"position":{"line":5,"character":9}}}"#,
    );
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The class Ticket declaration starts at line 1, column 0. (serde_json sorts
    // object keys alphabetically, so `character` precedes `line`.)
    assert!(
        stdout.contains(r#""start":{"character":0,"line":1}"#),
        "definition should resolve to the class declaration: {stdout}"
    );
}

#[test]
fn lsp_cross_file_definition_and_workspace_symbol() {
    // With a workspace root, go-to-definition resolves a name declared in ANOTHER
    // file, and workspace/symbol indexes every `.whip` file on disk (not just open
    // documents). Here `b.whip` references `class Ticket` declared in `a.whip`.
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let dir =
        std::env::temp_dir().join(format!("whip-lsp-xfile-{}-{}", std::process::id(), line!()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp workspace dir");
    let a = dir.join("a.whip");
    let b = dir.join("b.whip");
    fs::write(&a, "workflow Demo\nclass Ticket {\n  id string\n}\n").expect("write a.whip");
    fs::write(&b, "rule r\n  when Ticket as t\n=> {\n  done t\n}\n").expect("write b.whip");
    let root_uri = format!("file://{}", dir.display());
    let a_uri = format!("file://{}", a.display());
    let b_uri = format!("file://{}", b.display());
    // didOpen b.whip with its real content (the cursor file must be in `documents`).
    let b_text = "rule r\\n  when Ticket as t\\n=> {\\n  done t\\n}\\n";

    let mut input = String::new();
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"rootUri":"{root_uri}"}}}}"#
    ));
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{b_uri}","text":"{b_text}"}}}}}}"#
    ));
    // definition on the `Ticket` reference in b.whip (line 1, within `Ticket`).
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/definition","params":{{"textDocument":{{"uri":"{b_uri}"}},"position":{{"line":1,"character":9}}}}}}"#
    ));
    // workspace/symbol for "Ticket" — found in a.whip, which was never opened.
    input += &frame(
        r#"{"jsonrpc":"2.0","id":3,"method":"workspace/symbol","params":{"query":"Ticket"}}"#,
    );
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Cross-file definition resolves to a.whip's URI (the declaring file).
    assert!(
        stdout.contains(&format!("\"uri\":\"{a_uri}\"")),
        "cross-file definition should resolve to a.whip: {stdout}"
    );
    // workspace/symbol indexed a.whip on disk and found Ticket there.
    assert!(
        stdout.contains("\"name\":\"Ticket\""),
        "workspace/symbol should index Ticket from a.whip on disk: {stdout}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn lsp_hover_shows_declaration_source() {
    // `textDocument/hover` on a reference shows the target declaration's source, so
    // hovering the `Ticket` reference reveals `class Ticket { ... }`.
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let text = "workflow Demo\\nclass Ticket {\\n  id string\\n}\\nrule r\\n  when Ticket as t\\n=> {\\n  done t\\n}\\n";
    let mut input = String::new();
    input += &frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///h.whip","text":"{text}"}}}}}}"#
    ));
    input += &frame(
        r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///h.whip"},"position":{"line":5,"character":9}}}"#,
    );
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("class Ticket {"),
        "hover should show the class declaration source: {stdout}"
    );
}

#[test]
fn lsp_completion_offers_keywords_and_declared_symbols() {
    // `textDocument/completion` returns a flat candidate list: language keywords
    // plus the document's declared top-level names (editors filter by prefix).
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let text = "workflow Demo\\nclass Ticket {\\n  id string\\n}\\n";
    let mut input = String::new();
    input += &frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///c.whip","text":"{text}"}}}}}}"#
    ));
    input += &frame(
        r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///c.whip"},"position":{"line":3,"character":0}}}"#,
    );
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // A keyword and the declared class both appear as completion labels.
    assert!(
        stdout.contains(r#""label":"rule""#),
        "completion offers keywords: {stdout}"
    );
    assert!(
        stdout.contains(r#""label":"Ticket""#),
        "completion offers declared symbols: {stdout}"
    );
}

#[test]
fn lsp_find_references_lists_all_occurrences() {
    // `textDocument/references` returns every whole-token occurrence of the
    // top-level symbol under the cursor (here `Ticket`: its declaration on line 1
    // and the `when Ticket` reference on line 5).
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let text = "workflow Demo\\nclass Ticket {\\n  id string\\n}\\nrule r\\n  when Ticket as t\\n=> {\\n  done t\\n}\\n";
    let mut input = String::new();
    input += &frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///r.whip","text":"{text}"}}}}}}"#
    ));
    input += &frame(
        r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/references","params":{"textDocument":{"uri":"file:///r.whip"},"position":{"line":5,"character":9},"context":{"includeDeclaration":true}}}"#,
    );
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The references response (id 2) carries both occurrences (lines 1 and 5).
    assert!(
        stdout.contains(r#""start":{"character":6,"line":1}"#),
        "references include the declaration occurrence: {stdout}"
    );
    assert!(
        stdout.contains(r#""start":{"character":7,"line":5}"#),
        "references include the use occurrence: {stdout}"
    );
}

#[test]
fn lsp_rename_edits_code_occurrences_but_not_strings() {
    // `textDocument/rename` edits every code occurrence of the symbol but must NOT
    // touch the same word inside a prompt string (that would corrupt content). Here
    // `Ticket` appears twice in code (declaration + `when`) and once in a prompt;
    // only the two code occurrences are renamed.
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let text = "workflow Demo\\nclass Ticket {\\n  id string\\n}\\nagent a {\\n  provider fixture\\n  capacity 1\\n}\\nrule r\\n  when Ticket as t\\n  when a is available\\n=> {\\n  tell a \\\"\\\"\\\"Look at this Ticket now.\\\"\\\"\\\"\\n  done t\\n}\\n";
    let mut input = String::new();
    input += &frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///rn.whip","text":"{text}"}}}}}}"#
    ));
    input += &frame(
        r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/rename","params":{"textDocument":{"uri":"file:///rn.whip"},"position":{"line":9,"character":9},"newName":"Issue"}}"#,
    );
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Exactly two edits (declaration + the `when` reference) — the occurrence inside
    // the prompt string is excluded.
    let edit_count = stdout.matches(r#""newText":"Issue""#).count();
    assert_eq!(
        edit_count, 2,
        "rename should edit only the two code occurrences: {stdout}"
    );
}

#[test]
fn lsp_formatting_returns_whole_document_edit() {
    // `textDocument/formatting` formats via the comment-preserving formatter and
    // returns a whole-document edit when the source is not already canonical.
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    // Six-space field indent — non-canonical, so formatting yields an edit.
    let text = "workflow Demo\\nclass Ticket {\\n      id string\\n}\\n";
    let mut input = String::new();
    input += &frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///f.whip","text":"{text}"}}}}}}"#
    ));
    input += &frame(
        r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/formatting","params":{"textDocument":{"uri":"file:///f.whip"},"options":{"tabSize":2,"insertSpaces":true}}}"#,
    );
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The formatting edit normalizes the field to a 2-space indent.
    assert!(
        stdout.contains(r#"  id string"#),
        "formatting should normalize the indentation: {stdout}"
    );
}

#[test]
fn lsp_document_highlight_marks_all_occurrences() {
    // `textDocument/documentHighlight` marks every occurrence of the symbol under
    // the cursor (here `Ticket`: its declaration and the `when` reference).
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let text = "workflow Demo\\nclass Ticket {\\n  id string\\n}\\nrule r\\n  when Ticket as t\\n=> {\\n  done t\\n}\\n";
    let mut input = String::new();
    input += &frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///hl.whip","text":"{text}"}}}}}}"#
    ));
    input += &frame(
        r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/documentHighlight","params":{"textDocument":{"uri":"file:///hl.whip"},"position":{"line":5,"character":9}}}"#,
    );
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Two highlights (declaration line 1 and use line 5), as Text highlights.
    let highlight_count = stdout.matches(r#""kind":1"#).count();
    assert!(
        highlight_count >= 2,
        "documentHighlight should mark both occurrences: {stdout}"
    );
}

#[test]
fn lsp_workspace_symbol_indexes_open_documents() {
    // `workspace/symbol` returns matching symbols across every OPEN document. Here
    // two documents are open; an empty query returns symbols from both, proving the
    // index spans documents rather than a single file.
    let bin = env!("CARGO_BIN_EXE_whip");
    let frame = |body: &str| format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let text_a = "workflow A\\nclass Ticket {\\n  id string\\n}\\n";
    let text_b = "workflow B\\nclass Order {\\n  id string\\n}\\n";
    let mut input = String::new();
    input += &frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///a.whip","text":"{text_a}"}}}}}}"#
    ));
    input += &frame(&format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///b.whip","text":"{text_b}"}}}}}}"#
    ));
    // Empty query → all symbols across both open documents.
    input +=
        &frame(r#"{"jsonrpc":"2.0","id":2,"method":"workspace/symbol","params":{"query":""}}"#);
    // Targeted query → only the matching symbol from b.whip.
    input += &frame(
        r#"{"jsonrpc":"2.0","id":3,"method":"workspace/symbol","params":{"query":"order"}}"#,
    );
    input += &frame(r#"{"jsonrpc":"2.0","method":"exit","params":{}}"#);

    let mut child = Command::new(bin)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn whip lsp");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input.as_bytes())
        .expect("write LSP input");
    let output = child.wait_with_output().expect("lsp exits");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Empty query surfaces symbols from BOTH documents.
    assert!(
        stdout.contains(r#""name":"Ticket""#) && stdout.contains("file:///a.whip"),
        "workspace/symbol should index document a: {stdout}"
    );
    assert!(
        stdout.contains(r#""name":"Order""#) && stdout.contains("file:///b.whip"),
        "workspace/symbol should index document b: {stdout}"
    );
    // The targeted `order` query response must not include `Ticket` (it has no `o`).
    let order_response = stdout
        .rsplit(r#""id":3"#)
        .next()
        .expect("workspace/symbol query response");
    assert!(
        !order_response.contains(r#""name":"Ticket""#),
        "case-insensitive query should filter out non-matching symbols: {stdout}"
    );
}

#[test]
fn test_harness_supports_per_agent_stub_outcomes() {
    // One scenario can stub different agents differently: `alpha` succeeds while
    // `beta` fails. The succeeding agent's turn completes (its observing rule
    // fires and records); the failing agent's turn does not (its rule never
    // fires) — both `agent.tell completed` and `agent.tell failed` are present.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-per-agent");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
        r#"
@service
workflow TwoAgents

class AlphaDone {
  summary string
}

class BetaDone {
  summary string
}

agent alpha {
  provider fixture
  profile "repo-writer"
  capacity 1
}

agent beta {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule tell_alpha
  when started
  when alpha is available
=> {
  tell alpha "Do alpha."
}

rule tell_beta
  when started
  when beta is available
=> {
  tell beta "Do beta."
}

rule observe_alpha
  when alpha completed turn as turn
=> {
  record AlphaDone {
    summary turn.summary
  }
}

rule observe_beta
  when beta completed turn as turn
=> {
  record BetaDone {
    summary turn.summary
  }
}

test "alpha succeeds, beta fails" {
  workflow TwoAgents
  stub agent alpha succeeds
  stub agent beta fails
  run until idle
  expect rule observe_alpha fired
  expect AlphaDone exists
  expect rule observe_beta did not fire
  expect effect agent.tell completed
  expect effect agent.tell failed
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let status = report
        .get("scenarios")
        .and_then(Value::as_array)
        .and_then(|scenarios| scenarios.first())
        .and_then(|scenario| scenario.get("status"))
        .and_then(Value::as_str);
    assert_eq!(
        status,
        Some("passed"),
        "per-agent outcomes report: {report}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_harness_stub_settles_agent_turns_and_outcome_changes_behavior() {
    // The harness drives queued effects through the fixture provider, with the
    // `stub` outcome controlling settlement. A `succeeds` stub lets the agent
    // turn complete (so the observing rule fires and the workflow completes);
    // a `fails` stub settles the turn as failed (so the observing rule never
    // fires and the workflow stays running) — proving the outcome is not
    // cosmetic.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-stub");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
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

test "succeeds stub completes the turn" {
  workflow CompletedTurn
  stub agent worker succeeds
  run until idle
  expect workflow completed
  expect rule observe fired
  expect TurnSeen exists
}

test "fails stub blocks completion" {
  workflow CompletedTurn
  stub agent worker fails
  run until idle
  expect rule observe did not fire
  expect workflow completed
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios array");
    let scenario = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                // Scenario ids are `<workflow>::<name>`; these single-workflow
                // fixtures look up by the trailing name.
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .unwrap_or_else(|| panic!("scenario {id} present"))
    };
    let status_of = |id: &str| {
        scenario(id)
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("missing")
            .to_owned()
    };
    // succeeds: every expectation holds.
    assert_eq!(status_of("succeeds stub completes the turn"), "passed");
    // fails: the observing rule genuinely does not fire (that expectation
    // passes), but the workflow does not complete (that expectation fails) — so
    // the scenario as a whole fails, demonstrating `fails` != `succeeds`.
    let fails = scenario("fails stub blocks completion");
    assert_eq!(fails.get("status").and_then(Value::as_str), Some("failed"));
    let expectations = fails
        .get("expectations")
        .and_then(Value::as_array)
        .expect("expectations array");
    let expect_status = |description: &str| {
        expectations
            .iter()
            .find(|e| e.get("description").and_then(Value::as_str) == Some(description))
            .and_then(|e| e.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
    };
    assert_eq!(expect_status("rule observe did not fire"), "passed");
    assert_eq!(expect_status("workflow completed"), "failed");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_harness_projects_over_dotted_runtime_facts() {
    // Projection nouns are dotted fact names, so a scenario can assert over
    // runtime facts like `agent.turn.completed`, not just single-identifier user
    // facts. Matching is exact: a failed turn produces `agent.turn.failed`, so
    // `agent.turn.completed exists` must fail there — never a substring match.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-dotted");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
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

test "dotted runtime fact projection" {
  workflow CompletedTurn
  stub agent worker succeeds
  run until idle
  expect agent.turn.completed exists
  expect agent.turn.completed where status == "completed"
  expect agent.turn.completed where agent == "worker"
  expect TurnSeen exists
}

test "dotted matching is exact" {
  workflow CompletedTurn
  stub agent worker fails
  run until idle
  expect agent.turn.failed exists
  expect agent.turn.completed exists
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios");
    let scenario = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                // Scenario ids are `<workflow>::<name>`; these single-workflow
                // fixtures look up by the trailing name.
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .unwrap_or_else(|| panic!("scenario {id} present"))
    };
    // All dotted projections over the completed turn hold.
    assert_eq!(
        scenario("dotted runtime fact projection")
            .get("status")
            .and_then(Value::as_str),
        Some("passed")
    );
    // Exact matching: the failed-turn scenario passes `agent.turn.failed exists`
    // but fails `agent.turn.completed exists`.
    let exact = scenario("dotted matching is exact");
    assert_eq!(exact.get("status").and_then(Value::as_str), Some("failed"));
    let expectations = exact
        .get("expectations")
        .and_then(Value::as_array)
        .expect("expectations");
    let expect_status = |description: &str| {
        expectations
            .iter()
            .find(|e| e.get("description").and_then(Value::as_str) == Some(description))
            .and_then(|e| e.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
    };
    assert_eq!(expect_status("agent.turn.failed exists"), "passed");
    assert_eq!(expect_status("agent.turn.completed exists"), "failed");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_harness_evaluates_diagnostic_expects() {
    // `expect diagnostic <code>` matches a runtime diagnostic recorded during
    // the run. A `fails` stub makes the fixture provider record a terminal
    // diagnostic with code `nonzero_exit`; a `succeeds` run records none, so the
    // same expectation must fail there — never a false pass.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-diag");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
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

test "failed turn emits nonzero_exit diagnostic" {
  workflow CompletedTurn
  stub agent worker fails
  run until idle
  expect diagnostic nonzero_exit
}

test "successful run records no such diagnostic" {
  workflow CompletedTurn
  stub agent worker succeeds
  run until idle
  expect diagnostic nonzero_exit
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios");
    let status_of = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                // Scenario ids are `<workflow>::<name>`; these single-workflow
                // fixtures look up by the trailing name.
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .and_then(|s| s.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
            .to_owned()
    };
    assert_eq!(
        status_of("failed turn emits nonzero_exit diagnostic"),
        "passed"
    );
    assert_eq!(
        status_of("successful run records no such diagnostic"),
        "failed"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_harness_run_for_n_steps_bounds_execution() {
    // `run for N steps` runs exactly N drive rounds (drain rules + settle
    // effects), not to a fixed point. In CompletedTurn, round 1 settles the
    // agent turn but the observing rule (and completion) only fire in round 2,
    // so `run for 1 steps` leaves the workflow running — proving the bound is
    // real and not silently run-to-idle.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-steps");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
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

test "one step settles the turn but not completion" {
  workflow CompletedTurn
  stub agent worker succeeds
  run for 1 steps
  expect rule begin fired
  expect effect agent.tell completed
  expect rule observe did not fire
}

test "two steps complete the workflow" {
  workflow CompletedTurn
  stub agent worker succeeds
  run for 2 steps
  expect workflow completed
  expect TurnSeen exists
}

test "one step is not yet complete" {
  workflow CompletedTurn
  stub agent worker succeeds
  run for 1 steps
  expect workflow completed
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios");
    let status_of = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                // Scenario ids are `<workflow>::<name>`; these single-workflow
                // fixtures look up by the trailing name.
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .and_then(|s| s.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
            .to_owned()
    };
    assert_eq!(
        status_of("one step settles the turn but not completion"),
        "passed"
    );
    assert_eq!(status_of("two steps complete the workflow"), "passed");
    // The bound is real: one step does not reach completion.
    assert_eq!(status_of("one step is not yet complete"), "failed");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_harness_evaluates_effect_and_no_effect_expects() {
    // `expect effect <kind> requested|completed|failed` and `expect no <kind>`
    // read the settled effect log. A `succeeds` stub completes the agent turn
    // (so `agent.tell` is requested and completed, and no `script.run` exists);
    // a `fails` stub leaves `agent.tell` failed; and `expect no agent.tell` when
    // one was requested must fail — never a false pass.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-effect");
    let wf = dir.join("wf.whip");
    fs::write(
        &wf,
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

test "succeeds settles agent.tell completed" {
  workflow CompletedTurn
  stub agent worker succeeds
  run until idle
  expect effect agent.tell requested
  expect effect agent.tell completed
  expect no script.run
}

test "fails settles agent.tell failed" {
  workflow CompletedTurn
  stub agent worker fails
  run until idle
  expect effect agent.tell failed
}

test "expect no agent.tell is false here" {
  workflow CompletedTurn
  stub agent worker succeeds
  run until idle
  expect no agent.tell
}
"#,
    )
    .expect("write workflow");

    let output = Command::new(bin)
        .args(["--json", "test", wf.to_str().expect("present")])
        .output()
        .expect("whip test runs");
    let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
    let scenarios = report
        .get("scenarios")
        .and_then(Value::as_array)
        .expect("scenarios");
    let status_of = |id: &str| {
        scenarios
            .iter()
            .find(|s| {
                // Scenario ids are `<workflow>::<name>`; these single-workflow
                // fixtures look up by the trailing name.
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .and_then(|s| s.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
            .to_owned()
    };
    assert_eq!(status_of("succeeds settles agent.tell completed"), "passed");
    assert_eq!(status_of("fails settles agent.tell failed"), "passed");
    assert_eq!(status_of("expect no agent.tell is false here"), "failed");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_harness_seeds_given_fact_and_given_input() {
    // `given fact` seeds a pre-existing fact and `given input` seeds the
    // workflow input (validated against the input contract and derived as the
    // declared input fact). Both must flow through the guard: a value that
    // matches fires the rule; a value that does not is filtered.
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("harness-given");

    // `given fact`: workflow with no input contract, rule keyed on a fact.
    let fact_wf = dir.join("fact.whip");
    fs::write(
        &fact_wf,
        r#"
workflow FactDriven

output result Done

class Ticket {
  status string
}

class Done {
  status string
}

rule on_ticket
  when Ticket as t where t.status == "open"
=> {
  record Done {
    status t.status
  }
  complete result {
    status t.status
  }
}

test "matching seeded fact fires" {
  workflow FactDriven
  given fact Ticket {
    status "open"
  }
  run until idle
  expect workflow completed
  expect rule on_ticket fired
  expect Done where status == "open"
}

test "non-matching seeded fact is filtered" {
  workflow FactDriven
  given fact Ticket {
    status "closed"
  }
  run until idle
  expect rule on_ticket did not fire
}
"#,
    )
    .expect("write fact workflow");

    // `given input`: workflow with an input contract; input seeds the fact.
    let input_wf = dir.join("input.whip");
    fs::write(
        &input_wf,
        r#"
workflow InputDriven

input ticket Ticket
output result Done

class Ticket {
  status string
}

class Done {
  status string
}

rule on_ticket
  when Ticket as t where t.status == "open"
=> {
  record Done {
    status t.status
  }
  complete result {
    status t.status
  }
}

test "seeded input drives the workflow" {
  workflow InputDriven
  given input {
    ticket { status "open" }
  }
  run until idle
  expect workflow completed
  expect Done where status == "open"
}

test "input violating the contract is invalid" {
  workflow InputDriven
  given input {
    wrongkey { status "open" }
  }
  run until idle
  expect workflow completed
}
"#,
    )
    .expect("write input workflow");

    let status_of = |path: &std::path::Path, id: &str| {
        let output = Command::new(bin)
            .args(["--json", "test", path.to_str().expect("present")])
            .output()
            .expect("whip test runs");
        let report: Value = serde_json::from_slice(&output.stdout).expect("test report JSON");
        report
            .get("scenarios")
            .and_then(Value::as_array)
            .expect("scenarios")
            .iter()
            .find(|s| {
                // Scenario ids are `<workflow>::<name>`; these single-workflow
                // fixtures look up by the trailing name.
                s.get("id")
                    .and_then(Value::as_str)
                    .and_then(|full| full.rsplit("::").next())
                    == Some(id)
            })
            .and_then(|s| s.get("status").and_then(Value::as_str))
            .unwrap_or("missing")
            .to_owned()
    };

    assert_eq!(status_of(&fact_wf, "matching seeded fact fires"), "passed");
    assert_eq!(
        status_of(&fact_wf, "non-matching seeded fact is filtered"),
        "passed"
    );
    assert_eq!(
        status_of(&input_wf, "seeded input drives the workflow"),
        "passed"
    );
    // A `given input` that violates the input contract cannot run faithfully.
    assert_eq!(
        status_of(&input_wf, "input violating the contract is invalid"),
        "invalid"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fmt_preserves_placeable_comments_and_refuses_unplaceable_ones() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("fmt");

    // A comment-free file in non-canonical form: `fmt --check` reports it, then
    // `fmt` rewrites it, then it is idempotent.
    let plain = dir.join("plain.whip");
    fs::write(&plain, "workflow Demo\nclass Task {\n  title string\n}\n").expect("write plain");
    let check = Command::new(bin)
        .args(["fmt", "--check", plain.to_str().expect("present")])
        .output()
        .expect("fmt --check runs");
    assert!(
        !check.status.success(),
        "non-canonical file should fail --check"
    );

    let format = Command::new(bin)
        .args(["fmt", plain.to_str().expect("present")])
        .output()
        .expect("fmt runs");
    assert!(
        format.status.success(),
        "fmt should succeed on a comment-free file"
    );
    let recheck = Command::new(bin)
        .args(["fmt", "--check", plain.to_str().expect("present")])
        .output()
        .expect("fmt --check runs");
    assert!(
        recheck.status.success(),
        "formatted file should be idempotent"
    );

    // A trailing comment on a declaration's opening-brace line has no member on
    // that line to attach to, so it is refused without modification — never
    // silently lost. This exercises the no-loss count guard for a position the
    // body interleave cannot place.
    let commented = dir.join("commented.whip");
    let original =
        "# keep me\nworkflow Demo2\n\nclass Task {  # header comment\n  title string\n}\n";
    fs::write(&commented, original).expect("write commented");
    let refused = Command::new(bin)
        .args(["fmt", commented.to_str().expect("present")])
        .output()
        .expect("fmt runs");
    assert!(
        !refused.status.success(),
        "fmt must refuse a comment it cannot place"
    );
    assert!(String::from_utf8_lossy(&refused.stderr).contains("skipped to avoid data loss"));
    assert_eq!(
        fs::read_to_string(&commented).expect("read commented"),
        original,
        "fmt must not modify a file it refuses"
    );

    // A TOP-LEVEL trailing comment on a single-line declaration (e.g.
    // `workflow Demo  # ...`) is now preserved by attaching it to that element's
    // line, and the result is idempotent.
    let top_trailing = dir.join("top_trailing.whip");
    fs::write(
        &top_trailing,
        "workflow Demo8  # the entry point\n\nclass Task {\n  title string\n}\n",
    )
    .expect("write top_trailing");
    let top_fmt = Command::new(bin)
        .args(["fmt", top_trailing.to_str().expect("present")])
        .output()
        .expect("fmt runs");
    assert!(
        top_fmt.status.success(),
        "fmt should preserve a top-level trailing comment: {}",
        String::from_utf8_lossy(&top_fmt.stderr)
    );
    assert!(
        fs::read_to_string(&top_trailing)
            .expect("read top_trailing")
            .contains("workflow Demo8  # the entry point"),
        "top-level trailing comment was dropped"
    );
    let top_recheck = Command::new(bin)
        .args(["fmt", "--check", top_trailing.to_str().expect("present")])
        .output()
        .expect("fmt --check runs");
    assert!(
        top_recheck.status.success(),
        "top-level trailing comment placement must be idempotent"
    );

    // Top-level LEADING comments (own-line `#`/`//` above a declaration, or a
    // file header) ARE preserved, and the result is idempotent.
    let leading = dir.join("leading.whip");
    fs::write(
        &leading,
        "# file header\nworkflow Demo4\n# explains the class\nclass Task {\n  title string\n}\n",
    )
    .expect("write leading");
    let leading_fmt = Command::new(bin)
        .args(["fmt", leading.to_str().expect("present")])
        .output()
        .expect("fmt runs");
    assert!(
        leading_fmt.status.success(),
        "fmt should preserve top-level leading comments: {}",
        String::from_utf8_lossy(&leading_fmt.stderr)
    );
    let leading_out = fs::read_to_string(&leading).expect("read leading");
    assert!(
        leading_out.contains("# file header") && leading_out.contains("# explains the class"),
        "leading comments were dropped:\n{leading_out}"
    );
    let leading_recheck = Command::new(bin)
        .args(["fmt", "--check", leading.to_str().expect("present")])
        .output()
        .expect("fmt --check runs");
    assert!(
        leading_recheck.status.success(),
        "comment-preserving formatting must be idempotent"
    );

    // A comment INSIDE a rule body is preserved (the body's raw text carries it).
    let body_comment = dir.join("body_comment.whip");
    fs::write(
        &body_comment,
        "workflow Demo5\noutput result D\nclass D {\n  x string\n}\nrule r\n  when started\n=> {\n  # note inside the body\n  complete result {\n    x \"y\"\n  }\n}\n",
    )
    .expect("write body_comment");
    let body_fmt = Command::new(bin)
        .args(["fmt", body_comment.to_str().expect("present")])
        .output()
        .expect("fmt runs");
    assert!(
        body_fmt.status.success(),
        "fmt should preserve a rule-body comment: {}",
        String::from_utf8_lossy(&body_fmt.stderr)
    );
    assert!(
        fs::read_to_string(&body_comment)
            .expect("read body_comment")
            .contains("# note inside the body"),
        "rule-body comment was dropped"
    );

    // Own-line AND trailing comments inside CLASS/AGENT/ENUM/SIGNAL/QUEUE bodies
    // (whose formatters rebuild from the AST) are preserved — own-line ones
    // interleaved by source position, trailing ones appended to their member's line
    // — and the result is idempotent.
    let class_comment = dir.join("class_comment.whip");
    let class_src = concat!(
        "@service\n",
        "workflow Demo6\n",
        "class Task {\n  # describe the field\n  title string  # trailing field\n}\n",
        "agent worker {\n  # which provider\n  provider fixture  # trailing provider\n  capacity 1\n}\n",
        "enum Status {\n  # accepted\n  Accept  # trailing accept\n  # rejected\n  Reject\n}\n",
        "signal deploy.finished {\n  # the service\n  service string  # trailing service\n  status string\n}\n",
        "tracker tickets {\n  # the backend\n  provider builtin  # builtin only\n}\n",
        "file store docs {\n  # the root dir\n  root \"./docs\"\n  allow read [\"*.md\"]  # markdown only\n}\n",
    );
    fs::write(&class_comment, class_src).expect("write class_comment");
    let class_fmt = Command::new(bin)
        .args(["fmt", class_comment.to_str().expect("present")])
        .output()
        .expect("fmt runs");
    assert!(
        class_fmt.status.success(),
        "fmt should preserve own-line and trailing class/agent/enum/signal/queue body comments: {}",
        String::from_utf8_lossy(&class_fmt.stderr)
    );
    let class_out = fs::read_to_string(&class_comment).expect("read class_comment");
    for marker in [
        "# describe the field",
        "# which provider",
        "# accepted",
        "# rejected",
        "# the service",
        "# the backend",
        "# the root dir",
        "allow read [\"*.md\"]  # markdown only",
        "title string  # trailing field",
        "provider fixture  # trailing provider",
        "Accept  # trailing accept",
        "service string  # trailing service",
        "provider builtin  # builtin only",
    ] {
        assert!(
            class_out.contains(marker),
            "body comment `{marker}` was dropped:\n{class_out}"
        );
    }
    let class_recheck = Command::new(bin)
        .args(["fmt", "--check", class_comment.to_str().expect("present")])
        .output()
        .expect("fmt --check runs");
    assert!(
        class_recheck.status.success(),
        "class/agent/enum comment interleaving must be idempotent"
    );

    // Comments INSIDE a data-carrying enum variant's nested field block are
    // preserved too (own-line interleaved, trailing appended) — the block is a
    // field list in braces, classified one level deeper — and stay idempotent.
    let nested = dir.join("nested_enum.whip");
    let nested_src = "workflow Demo7\nenum E {\n  Data {\n    # the id\n    id string\n    score int  # the score\n  }\n}\n";
    fs::write(&nested, nested_src).expect("write nested_enum");
    let nested_fmt = Command::new(bin)
        .args(["fmt", nested.to_str().expect("present")])
        .output()
        .expect("fmt runs");
    assert!(
        nested_fmt.status.success(),
        "fmt should preserve nested enum-variant comments: {}",
        String::from_utf8_lossy(&nested_fmt.stderr)
    );
    let nested_out = fs::read_to_string(&nested).expect("read nested_enum");
    assert!(
        nested_out.contains("# the id") && nested_out.contains("score int  # the score"),
        "nested enum-variant comments were dropped:\n{nested_out}"
    );
    let nested_recheck = Command::new(bin)
        .args(["fmt", "--check", nested.to_str().expect("present")])
        .output()
        .expect("fmt --check runs");
    assert!(
        nested_recheck.status.success(),
        "nested enum-variant comment placement must be idempotent"
    );

    // A rule body with a multi-line `"""..."""` string formats idempotently and
    // WITHOUT corrupting the string content: the content keeps its relative
    // indentation (dedented then re-indented to the block depth), and a second
    // pass changes nothing.
    let multiline = dir.join("multiline.whip");
    let multiline_src = concat!(
        "workflow Demo3\n",
        "agent worker {\n  provider fixture\n  profile \"x\"\n  capacity 1\n}\n",
        "rule begin\n  when started\n  when worker is available\n=> {\n",
        "  tell worker \"\"\"markdown\n  # Heading\n    indented\n  \"\"\"\n}\n",
    );
    fs::write(&multiline, multiline_src).expect("write multiline");
    let format_ml = Command::new(bin)
        .args(["fmt", multiline.to_str().expect("present")])
        .output()
        .expect("fmt runs");
    assert!(
        format_ml.status.success(),
        "fmt should format a multi-line-string rule body: {}",
        String::from_utf8_lossy(&format_ml.stderr)
    );
    let formatted_ml = fs::read_to_string(&multiline).expect("read multiline");
    // The string content (and its relative indentation) is preserved, not grown.
    assert!(
        formatted_ml.contains("  # Heading\n    indented\n"),
        "fmt corrupted multi-line string content:\n{formatted_ml}"
    );
    let recheck_ml = Command::new(bin)
        .args(["fmt", "--check", multiline.to_str().expect("present")])
        .output()
        .expect("fmt --check runs");
    assert!(
        recheck_ml.status.success(),
        "multi-line-string formatting must be idempotent"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fmt_is_non_destructive_across_every_example() {
    // Corpus-wide safety guarantee: for every shipped example, `whip fmt` must
    // either format it idempotently (a second `--check` passes) or refuse it
    // (exit non-zero) leaving the file byte-identical. It must never write output
    // that drifts or corrupts content. This locks in the idempotency self-check —
    // AND a semantic-equivalence check: a formatted file must compile to the same
    // rule/class structure as the original. (Idempotency alone is insufficient: a
    // body-collapsing bug that emits a *stable* placeholder like `flow X { ... }`
    // passes the idempotency check while destroying the body. The structure check
    // catches that.)
    let bin = env!("CARGO_BIN_EXE_whip");
    let dir = unique_temp_dir("fmt-corpus");
    let examples_dir = format!("{}/../../examples", env!("CARGO_MANIFEST_DIR"));
    // Compile a file and project its IR to the sorted set of `rule`/`class` lines,
    // or `None` if it does not compile bare (e.g. needs `--root`/a lock).
    let compile_structure = |file: &str| -> Option<Vec<String>> {
        let out = Command::new(bin).args(["compile", file]).output().ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut lines: Vec<String> = text
            .lines()
            .filter(|line| line.starts_with("  rule ") || line.starts_with("  class "))
            .map(|line| line.to_owned())
            .collect();
        lines.sort();
        Some(lines)
    };
    // Examples that `include` a shared library resolve the include relative to the
    // formatted file's own directory, so the scratch dir must mirror the shared
    // `includes/` subtree — otherwise the formatted copy cannot compile bare and
    // `compile_structure` returns `None` against a `Some(...)` original, a false
    // "corruption" signal that has nothing to do with formatting.
    let includes_src = format!("{examples_dir}/includes");
    if let Ok(entries) = fs::read_dir(&includes_src) {
        let includes_dst = dir.join("includes");
        fs::create_dir_all(&includes_dst).expect("create scratch includes dir");
        for entry in entries.flatten() {
            let include_path = entry.path();
            if include_path.extension().and_then(|e| e.to_str()) == Some("whip") {
                let name = include_path.file_name().expect("include file name");
                fs::copy(&include_path, includes_dst.join(name)).expect("copy include library");
            }
        }
    }
    let mut checked = 0usize;
    for entry in fs::read_dir(&examples_dir).expect("read examples dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("whip") {
            continue;
        }
        let original = fs::read_to_string(&path).expect("read example");
        let scratch = dir.join(path.file_name().expect("file name"));
        fs::write(&scratch, &original).expect("write scratch");
        let scratch_str = scratch.to_str().expect("utf-8");

        let format = Command::new(bin)
            .args(["fmt", scratch_str])
            .output()
            .expect("fmt runs");
        if format.status.success() {
            // Formatted: a second pass must report nothing (idempotent).
            let recheck = Command::new(bin)
                .args(["fmt", "--check", scratch_str])
                .output()
                .expect("fmt --check runs");
            assert!(
                recheck.status.success(),
                "fmt formatted {} but the result is not idempotent",
                path.display()
            );
            // Semantic equivalence: where the original compiles bare, the formatted
            // file must compile to the same rule/class structure — no body lost.
            if let Some(original_structure) = compile_structure(path.to_str().expect("utf-8")) {
                let formatted_structure = compile_structure(scratch_str);
                assert_eq!(
                    Some(original_structure),
                    formatted_structure,
                    "fmt changed the compiled structure of {} (semantic corruption)",
                    path.display()
                );
            }
        } else {
            // Refused: the file must be untouched (no partial/corrupting write).
            assert_eq!(
                fs::read_to_string(&scratch).expect("read scratch"),
                original,
                "fmt refused {} but modified it",
                path.display()
            );
        }
        checked += 1;
    }
    assert!(checked > 0, "expected to check at least one example");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn check_discovers_lock_relative_to_workflow_file() {
    let bin = env!("CARGO_BIN_EXE_whip");
    // The lock lives beside the workflow, but the command runs from an unrelated
    // working directory with no lock in its ancestry. Discovery must walk up from
    // the workflow file's directory, not just the cwd.
    let neutral_cwd = unique_temp_dir("lock-relative-cwd");
    let project_dir = unique_temp_dir("lock-relative-project");
    let workflow_path = write_locked_recall_project(bin, &project_dir);

    let output = Command::new(bin)
        .current_dir(&neutral_cwd)
        .args(["check", workflow_path.to_str().expect("utf-8 workflow")])
        .output()
        .expect("check runs");
    assert!(
        output.status.success(),
        "check should discover the lock beside the workflow file\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_dir_all(&neutral_cwd);
    let _ = fs::remove_dir_all(&project_dir);
}

#[test]
fn check_lock_away_from_project_root_gives_actionable_hint() {
    // A package lock resolves its `source.path` relative to its own directory, so a
    // lock used away from the project root cannot reach its manifests. The error
    // should say so, not just "file not found".
    let bin = env!("CARGO_BIN_EXE_whip");
    let project = unique_temp_dir("lock-hint-project");
    let workflow = write_locked_recall_project(bin, &project);
    // Copy the generated lock into a sibling dir where `source.path` won't resolve.
    let elsewhere = unique_temp_dir("lock-hint-elsewhere");
    let misplaced = elsewhere.join("whip.lock");
    fs::copy(project.join("whip.lock"), &misplaced).expect("copy lock elsewhere");

    let output = Command::new(bin)
        .args([
            "check",
            "--package-lock",
            misplaced.to_str().expect("utf-8 lock"),
            workflow.to_str().expect("utf-8 workflow"),
        ])
        .output()
        .expect("check runs");
    assert!(!output.status.success(), "a misplaced lock fails the check");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("keep the lock at the project root"),
        "the error should hint at lock placement:\n{stderr}"
    );

    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&elsewhere);
}

#[test]
fn check_rejects_sources_implying_different_locks() {
    let bin = env!("CARGO_BIN_EXE_whip");
    // Two workflows under two project roots, each with its own lock. Checking both
    // at once cannot pick a single lock, so the command must fail and demand
    // `--package-lock`.
    let neutral_cwd = unique_temp_dir("lock-conflict-cwd");
    let project_a = unique_temp_dir("lock-conflict-a");
    let project_b = unique_temp_dir("lock-conflict-b");
    let workflow_a = write_locked_recall_project(bin, &project_a);
    let workflow_b = write_locked_recall_project(bin, &project_b);

    let output = Command::new(bin)
        .current_dir(&neutral_cwd)
        .args([
            "check",
            workflow_a.to_str().expect("utf-8 workflow a"),
            workflow_b.to_str().expect("utf-8 workflow b"),
        ])
        .output()
        .expect("check runs");
    assert!(
        !output.status.success(),
        "conflicting discovered locks must fail"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("imply different"),
        "expected a conflicting-lock diagnostic, got:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = fs::remove_dir_all(&neutral_cwd);
    let _ = fs::remove_dir_all(&project_a);
    let _ = fs::remove_dir_all(&project_b);
}

#[test]
fn check_resolves_embedded_memory_then_coexists_with_discovered_lock() {
    let bin = env!("CARGO_BIN_EXE_whip");
    // A project directory holding the workflow, its manifest copy, and the lock.
    let project_dir = {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "whipplescript-lock-discovery-{}-{nanos}",
            std::process::id()
        ))
    };
    fs::create_dir_all(&project_dir).expect("create project dir");
    let workflow_path = project_dir.join("wf.whip");
    fs::write(
        &workflow_path,
        r#"
@service
workflow LockDiscovery

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
  recall project_memory for item as context

  after context succeeds {
    tell worker """
    Use the recalled context for {{ item.title }}.
    """
  }
}
"#,
    )
    .expect("workflow writes");

    // No `whip.lock` at all: `memory` now ships as an embedded std manifest (M5),
    // so `use memory` + `recall` resolves from the binary itself — check passes
    // with no supply chain. (The no-lock guard for genuinely non-embedded packages
    // is covered by `package_lock_supplies_package_import_registry`.)
    let absent = Command::new(bin)
        .current_dir(&project_dir)
        .args(["check", "wf.whip"])
        .output()
        .expect("check runs");
    assert!(
        absent.status.success(),
        "embedded `memory` manifest must let check pass with no lock\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&absent.stdout),
        String::from_utf8_lossy(&absent.stderr)
    );

    // Place a portable `whip.lock` (with its manifest co-located) in the project
    // directory and re-run check WITHOUT `--package-lock`; discovery walks up from
    // the cwd and finds the lock. The lock and the embedded manifest must coexist
    // (the lock wins; no duplicate-registration conflict).
    let memory_manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packages/memory.json");
    let manifest_copy = project_dir.join("memory.json");
    fs::copy(&memory_manifest, &manifest_copy).expect("copy manifest beside lock");
    run_text(
        bin,
        &[
            "package",
            "lock",
            "--output",
            project_dir.join("whip.lock").to_str().expect("utf-8 lock"),
            manifest_copy.to_str().expect("utf-8 manifest"),
        ],
    );
    let discovered = Command::new(bin)
        .current_dir(&project_dir)
        .args(["check", "wf.whip"])
        .output()
        .expect("check runs");
    assert!(
        discovered.status.success(),
        "check should discover the nearest whip.lock without --package-lock\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&discovered.stdout),
        String::from_utf8_lossy(&discovered.stderr)
    );

    let _ = fs::remove_dir_all(&project_dir);
}

#[test]
fn tampered_manifest_fails_lock_load_with_stable_kind() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let project_dir = {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "whipplescript-lock-tamper-{}-{nanos}",
            std::process::id()
        ))
    };
    fs::create_dir_all(&project_dir).expect("create project dir");
    let workflow_path = project_dir.join("wf.whip");
    fs::write(
        &workflow_path,
        r#"
@service
workflow Tamper

use memory

class WorkItem {
  title string
}

agent worker {
  provider fixture
  profile "repo-writer"
  capacity 1
}

rule recall_before_work
  when WorkItem as item
  when worker is available
=> {
  recall project_memory for item as context

  after context succeeds {
    tell worker "{{ item.title }}"
  }
}
"#,
    )
    .expect("workflow writes");
    let memory_manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/packages/memory.json");
    let manifest_copy = project_dir.join("memory.json");
    fs::copy(&memory_manifest, &manifest_copy).expect("copy manifest beside lock");
    run_text(
        bin,
        &[
            "package",
            "lock",
            "--output",
            project_dir.join("whip.lock").to_str().expect("utf-8 lock"),
            manifest_copy.to_str().expect("utf-8 manifest"),
        ],
    );

    // Tamper the manifest bytes after locking; the lock pins it by SHA-256, which
    // is recomputed at load, so the mismatch must be rejected.
    let mut tampered = fs::read_to_string(&manifest_copy).expect("read manifest");
    tampered.push_str("\n ");
    fs::write(&manifest_copy, tampered).expect("write tampered manifest");

    let output = Command::new(bin)
        .current_dir(&project_dir)
        .args(["--json", "check", "wf.whip"])
        .output()
        .expect("check runs");
    assert!(
        !output.status.success(),
        "tampered manifest must fail lock load"
    );
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("check emits a JSON report array");
    let entry = report
        .as_array()
        .and_then(|entries| entries.first())
        .unwrap_or(&report);
    assert_eq!(
        entry.pointer("/error/kind").and_then(Value::as_str),
        Some("package_lock"),
        "tamper failure must carry the stable `package_lock` diagnostic kind, got: {report}"
    );
    assert!(
        entry
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("does not match manifest")),
        "tamper message should explain the manifest mismatch, got: {report}"
    );

    let _ = fs::remove_dir_all(&project_dir);
}

#[test]
fn dev_capability_call_validates_locked_package_output_schema() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("capability-call-output-validation");
    let manifest_path =
        temp_workflow_path("capability-call-output-validation-manifest").with_extension("json");
    let lock_path =
        temp_workflow_path("capability-call-output-validation-lock").with_extension("json");
    fs::write(
        &workflow_path,
        r#"
workflow CapabilityCallOutputValidation

use memory

class WorkItem {
  title string
}

rule seed
  when started
=> {
  record WorkItem {
    title "Remember this"
  }
}

rule recall
  when WorkItem as item
=> {
  call memory.query for item as context
}
"#,
    )
    .expect("workflow writes");
    fs::write(
        &manifest_path,
        r#"
{
  "schema": "whipplescript.package_manifest.v0",
  "package_id": "package-memory-bad-output",
  "name": "memory",
  "version": "0.1.0",
  "libraries": [
    {
      "id": "memory",
      "version": "0.1.0",
      "effect_contracts": [
        {
          "id": "memory.query",
          "effect_kind": "capability.call",
          "source_forms": ["call memory.query"],
          "input_schema": {"query": "string"},
          "output_schema": {"summary": "int", "target": "string"},
          "required_capabilities": ["memory.query"],
          "provider_kinds": ["memory-provider"],
          "validation": "runtime_boundary"
        }
      ]
    }
  ],
  "capabilities": [
    {
      "id": "memory.query",
      "description": "Query package memory.",
      "schema": {"input": {"query": "string"}}
    }
  ],
  "providers": [
    {
      "id": "provider-memory-query",
      "provider_kind": "memory-provider",
      "capability": "memory.query",
      "config": {}
    }
  ],
  "profiles": [
    {
      "id": "profile-memory-user",
      "name": "memory-user",
      "description": "Allow memory package queries.",
      "enforcement_mode": "enforce",
      "allowed_capabilities": ["memory.query"],
      "config": {}
    }
  ],
  "bindings": [
    {
      "id": "binding-memory-query-global",
      "program_id": null,
      "capability": "memory.query",
      "provider": "memory-provider",
      "config": {}
    }
  ]
}
"#,
    )
    .expect("manifest writes");
    run_text(
        bin,
        &[
            "package",
            "lock",
            "--output",
            lock_path.to_str().expect("utf-8 lock path"),
            manifest_path.to_str().expect("utf-8 manifest path"),
        ],
    );

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
            "--package-lock",
            lock_path.to_str().expect("utf-8 lock path"),
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
    assert!(facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("capability.call.failed")));
    assert!(!facts
        .iter()
        .any(|fact| fact.get("name").and_then(Value::as_str) == Some("capability.call.succeeded")));

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
    let runs = runs.as_array().expect("runs array");
    assert!(runs
        .iter()
        .any(|run| run.get("status").and_then(Value::as_str) == Some("failed")));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(lock_path);
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
    assert!(stderr.contains("was removed from the language"), "{stderr}");

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
fn dev_completes_a_scalar_terminal_payload() {
    // A workflow with a SCALAR output contract completes with a bare scalar value,
    // and the stored terminal payload is that scalar (not a field object).
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("workflow-scalar-complete");
    fs::write(
        &workflow_path,
        r#"
workflow ScalarComplete {
  output result float

  rule complete_immediately
    when started
  => {
    complete result 0.9
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
    let terminal = status.get("workflow_terminal").expect("terminal");
    assert_eq!(terminal.get("name").and_then(Value::as_str), Some("result"));
    // The persisted payload is the bare scalar value, not a `{ field: … }` object.
    assert_eq!(
        terminal.get("payload").and_then(Value::as_f64),
        Some(0.9),
        "scalar terminal payload not stored as a scalar: {terminal:?}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn dev_runs_included_bundle_with_explicit_root_selection() {
    // Phase 9 e2e: a source bundle that BOTH pulls in a library file via `include`
    // AND declares multiple workflows (so `--root` is required) runs deterministically
    // to completion. Exercises include resolution + root selection together in one dev run.
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let root = temp_workflow_path("include-root-dev");
    let lib = root.with_file_name(format!(
        "{}.lib.whip",
        root.file_stem()
            .expect("temp path has stem")
            .to_string_lossy()
    ));
    fs::write(
        &lib,
        r#"class SharedTicket {
  id string
}
"#,
    )
    .expect("write include lib");
    fs::write(
        &root,
        format!(
            r#"include "{}"

workflow Selected {{
  output result Out

  class Out {{
    id string
  }}

  rule go
    when started
  => {{
    record SharedTicket {{
      id "t-1"
    }}
    complete result {{
      id "t-1"
    }}
  }}
}}

workflow Other {{
  output result OtherOut

  class OtherOut {{
    note "other"
  }}

  rule go
    when started
  => {{
    complete result {{
      note "other"
    }}
  }}
}}
"#,
            lib.file_name().expect("lib file name").to_string_lossy()
        ),
    )
    .expect("write root bundle");

    let dev = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "dev",
            root.to_str().expect("utf-8 workflow path"),
            "--root",
            "Selected",
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
        Some("completed"),
        "included+root bundle should run to completion: {status}"
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(root);
    let _ = fs::remove_file(lib);
}

#[test]
fn status_surfaces_pending_human_asks_with_answer_command() {
    // An instance idle on an `askHuman` shows the pending ask and the exact command
    // (with available choices) to unblock it.
    let bin = env!("CARGO_BIN_EXE_whip");
    let store = temp_store_path();
    Command::new(bin)
        .args([
            "--store",
            store.to_str().expect("present"),
            "dev",
            example_path("triage-flow.whip").to_str().expect("present"),
            "--until",
            "idle",
        ])
        .output()
        .expect("dev runs");
    let instances = run_json(
        bin,
        &[
            "--store",
            store.to_str().expect("present"),
            "--json",
            "instances",
        ],
    );
    let instance_id = instances
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|i| i.get("instance_id"))
        .and_then(Value::as_str)
        .expect("instance id");

    let status = Command::new(bin)
        .args([
            "--store",
            store.to_str().expect("present"),
            "status",
            instance_id,
        ])
        .output()
        .expect("status runs");
    let out = String::from_utf8_lossy(&status.stdout);
    assert!(
        out.contains("pending human asks:")
            && out.contains("whip inbox answer ")
            && out.contains("--choice <approve|reject>"),
        "status should surface the pending ask:\n{out}"
    );
}

#[test]
fn dev_reports_the_final_instance_outcome() {
    // `whip dev` prints the final instance status so a run's result is visible without
    // a separate `whip status`. minimal-noop completes; triage-flow goes idle on a
    // human ask (still running) and says so.
    let bin = env!("CARGO_BIN_EXE_whip");

    let completed = Command::new(bin)
        .args([
            "--store",
            temp_store_path().to_str().expect("present"),
            "dev",
            example_path("minimal-noop.whip").to_str().expect("present"),
            "--until",
            "idle",
        ])
        .output()
        .expect("dev runs");
    assert!(
        String::from_utf8_lossy(&completed.stdout).contains("status completed"),
        "stdout:\n{}",
        String::from_utf8_lossy(&completed.stdout)
    );

    let idle = Command::new(bin)
        .args([
            "--store",
            temp_store_path().to_str().expect("present"),
            "dev",
            example_path("triage-flow.whip").to_str().expect("present"),
            "--until",
            "idle",
        ])
        .output()
        .expect("dev runs");
    let idle_out = String::from_utf8_lossy(&idle.stdout);
    assert!(
        idle_out.contains("status running") && idle_out.contains("awaiting a human answer"),
        "stdout:\n{idle_out}"
    );
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

    // A missing input names the expected type and shows the expected object shape, so
    // a caller who forgot the input-name nesting can see what to provide.
    let missing = Command::new(bin)
        .args([
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--input",
            r#"{"phaseId":"p3"}"#,
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--until",
            "idle",
        ])
        .output()
        .expect("command runs");
    assert!(!missing.status.success());
    let missing_stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(
        missing_stderr.contains("missing workflow input `phase` (expected `ref<PhaseRequest>`)")
            && missing_stderr.contains(r#"expected input object { "phase": <ref<PhaseRequest>> }"#),
        "{missing_stderr}"
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

/// Family C (child-milestone lifecycle, discriminated-families-design.md 7.3): a
/// child `emit milestone "<name>" of <Class>` projects a mid-flight milestone the
/// parent observes via `after child reaches "<name>" as m`, binding the milestone
/// payload, alongside the terminal — both through the unified after-machinery.
#[test]
fn dev_observes_child_milestone_via_reaches() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("workflow-milestone");
    fs::write(
        &workflow_path,
        r#"
workflow Parent {
  input task Task

  class Task {
    title string
  }

  class ParentSaw {
    note string
  }

  class ParentDone {
    title string
  }

  rule dispatch
    when Task as task
  => {
    invoke Child { task { title task.title } } as child

    after child reaches "halfway" as m {
      record ParentSaw {
        note m.note
      }
    }

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

  class Progress {
    note string
  }

  rule complete_child
    when Task as task
  => {
    emit milestone "halfway" of Progress {
      note task.title
    }

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
            r#"{"task":{"title":"Milestone smoke"}}"#,
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

    // The parent re-derived the milestone as a `reached` fact keyed by the
    // milestone name, carrying the child's projected payload.
    assert!(
        facts.iter().any(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("workflow.invoke.reached:halfway")
                && fact
                    .get("value")
                    .and_then(|value| value.get("value"))
                    .and_then(|value| value.get("note"))
                    .and_then(Value::as_str)
                    == Some("Milestone smoke")
        }),
        "expected a workflow.invoke.reached:halfway fact, facts: {facts:#?}"
    );

    // The `after child reaches "halfway" as m` arm fired and bound `m.note`.
    assert!(
        facts.iter().any(|fact| {
            fact.get("name").and_then(Value::as_str) == Some("ParentSaw")
                && fact
                    .get("value")
                    .and_then(|value| value.get("note"))
                    .and_then(Value::as_str)
                    == Some("Milestone smoke")
        }),
        "expected the reaches arm to fire (ParentSaw), facts: {facts:#?}"
    );

    // The terminal is still observed independently (defense in depth: milestone
    // observation does not displace terminal observation).
    assert!(facts.iter().any(|fact| {
        fact.get("name").and_then(Value::as_str) == Some("ParentDone")
            && fact
                .get("value")
                .and_then(|value| value.get("title"))
                .and_then(Value::as_str)
                == Some("Milestone smoke")
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
        Completed as result => {
          record ParentCancelled {
            reason result.title
          }
        }
        Failed as failure => {
          record ParentCancelled {
            reason failure.reason
          }
        }
        TimedOut as timeout => {
          record ParentCancelled {
            reason timeout.summary
          }
        }
        Cancelled as cancel => {
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
fn dev_provider_language_rehydrates_after_bound_coerce_arguments() {
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
    let coerce = evidence_items
        .iter()
        .find(|item| item.get("kind").and_then(Value::as_str) == Some("schema.coerce.provider"))
        .expect("coerce provider evidence");
    let arguments = coerce
        .get("metadata")
        .and_then(|metadata| metadata.get("arguments"))
        .expect("coerce arguments");
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
fn dev_provider_language_e2e_runs_agent_table_and_coerce_reviews() {
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
            .filter(
                |fact| fact.get("name").and_then(Value::as_str) == Some("schema.coerce.succeeded")
            )
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
      Completed as result => {
        record TerminalRoute {
          branch "completed"
          detail result.summary
        }
        askHuman "completed branch effect"
      }
      Failed as failure => {
        record TerminalRoute {
          branch "failed"
          detail failure.reason
        }
        askHuman "failed branch effect"
      }
      TimedOut as timeout => {
        record TerminalRoute {
          branch "timed_out"
          detail timeout.summary
        }
        askHuman "timed_out branch effect"
      }
      Cancelled as cancel => {
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
        .find(|item| item.get("kind").and_then(Value::as_str) == Some("schema.coerce.provider"))
        .and_then(|item| item.get("metadata"))
        .and_then(|metadata| metadata.get("arguments"))
        .expect("coerce arguments");
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
                    "facts": 2,
                    "effects": 1
                },
                "facts": [
                    {"name": "agent.turn.started", "count": 1},
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

// --- Observability / UX polish (Phase 7 + Phase 4) ---------------------------

#[test]
fn status_assembles_multi_level_invocation_tree() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("observability-invocation-tree");
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

  rule dispatch_grandchild
    when Task as task
  => {
    invoke GrandChild { task { title task.title } } as grandchild

    after grandchild succeeds as result {
      complete result {
        title result.title
      }
    }
  }
}

workflow GrandChild {
  input task Task
  output result GrandResult

  class Task {
    title string
  }

  class GrandResult {
    title string
  }

  rule finish
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
            r#"{"task":{"title":"deep"}}"#,
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

    // Level 1: parent -> child.
    let child = status
        .pointer("/invocation_tree/0")
        .expect("child invocation node");
    assert_eq!(
        child.get("target_workflow").and_then(Value::as_str),
        Some("Child")
    );
    let child_instance_id = child
        .get("child_instance_id")
        .and_then(Value::as_str)
        .expect("child instance id");

    // Level 2: child -> grandchild, nested under the child node's `children`.
    let grandchild = status
        .pointer("/invocation_tree/0/children/0")
        .expect("grandchild invocation node");
    assert_eq!(
        grandchild.get("target_workflow").and_then(Value::as_str),
        Some("GrandChild")
    );
    // The grandchild invocation's parent is the child instance (proving the
    // tree is genuinely multi-level, not a flattened sibling list).
    assert_eq!(
        grandchild.get("parent_instance_id").and_then(Value::as_str),
        Some(child_instance_id)
    );
    // The grandchild is a leaf: its own `children` array is present and empty.
    assert_eq!(
        grandchild
            .get("children")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn log_json_stamps_invocation_and_workflow_provenance() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("observability-log-provenance");
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
            r#"{"task":{"title":"trace me"}}"#,
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Parent",
            "--until",
            "idle",
        ],
    );
    let parent_instance_id = dev
        .get("instance_id")
        .and_then(Value::as_str)
        .expect("instance id");

    // Every parent event line is stamped with the workflow id + its own
    // instance id; a root workflow has no spawning invocation.
    let parent_log = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            parent_instance_id,
        ],
    );
    let parent_events = parent_log.as_array().expect("parent events array");
    assert!(!parent_events.is_empty());
    for event in parent_events {
        assert_eq!(
            event.get("instance_id").and_then(Value::as_str),
            Some(parent_instance_id)
        );
        assert!(event.get("workflow_id").and_then(Value::as_str).is_some());
        assert!(event.get("invocation_id").is_some());
        assert!(event.get("invocation_id").expect("key present").is_null());
    }

    // Discover the child instance and read the child invocation id from the
    // parent status invocation tree.
    let status = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "status",
            parent_instance_id,
        ],
    );
    let child_instance_id = status
        .pointer("/invocation_tree/0/child_instance_id")
        .and_then(Value::as_str)
        .expect("child instance id");
    let expected_invocation_id = status
        .pointer("/invocation_tree/0/invocation_id")
        .and_then(Value::as_str)
        .expect("child invocation id");

    // Every child event line is tied back to the invocation that spawned it.
    let child_log = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "log",
            child_instance_id,
        ],
    );
    let child_events = child_log.as_array().expect("child events array");
    assert!(!child_events.is_empty());
    for event in child_events {
        assert_eq!(
            event.get("instance_id").and_then(Value::as_str),
            Some(child_instance_id)
        );
        assert_eq!(
            event.get("invocation_id").and_then(Value::as_str),
            Some(expected_invocation_id)
        );
        assert!(event.get("workflow_id").and_then(Value::as_str).is_some());
    }

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}

#[test]
fn diagnostics_grouped_buckets_findings_by_provenance() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let v1 = temp_workflow_path("observability-diagnostics-v1");
    let v2 = temp_workflow_path("observability-diagnostics-v2");
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

    // A stale step against the pre-revision program path records a diagnostic.
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

    // Default output stays a flat array (backwards compatible).
    let flat = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "diagnostics",
            instance_id,
        ],
    );
    assert!(flat.is_array());

    // Grouped output buckets findings along file / workflow / subject-type.
    let grouped = run_json(
        bin,
        &[
            "--store",
            store_path.to_str().expect("utf-8 temp path"),
            "--json",
            "diagnostics",
            instance_id,
            "--grouped",
        ],
    );
    assert_eq!(
        grouped.get("schema").and_then(Value::as_str),
        Some("whipplescript.diagnostics_grouped.v0")
    );
    assert!(grouped.get("total").and_then(Value::as_u64).unwrap_or(0) >= 1);
    assert!(grouped
        .get("diagnostics")
        .and_then(Value::as_array)
        .is_some());
    let groups = grouped.get("groups").expect("groups object");
    // The stale-step diagnostic is subject_type `program_path`; it must appear
    // in that subject bucket and in the workflow (instance) bucket.
    let subject_bucket = groups
        .pointer("/by_subject_type/program_path")
        .and_then(Value::as_array)
        .expect("program_path subject bucket");
    assert!(subject_bucket.iter().any(|diagnostic| {
        diagnostic.get("code").and_then(Value::as_str) == Some("revision.stale_program_path")
    }));
    let workflow_bucket = groups
        .pointer(&format!("/by_workflow/{instance_id}"))
        .and_then(Value::as_array)
        .expect("workflow bucket keyed by instance id");
    assert!(!workflow_bucket.is_empty());
    assert!(groups
        .get("by_file")
        .and_then(Value::as_object)
        .map(|files| !files.is_empty())
        .unwrap_or(false));

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(v1);
    let _ = fs::remove_file(v2);
}

#[test]
fn status_failure_surface_separates_workflow_fail_from_provider_evidence() {
    let bin = env!("CARGO_BIN_EXE_whip");
    let store_path = temp_store_path();
    let workflow_path = temp_workflow_path("observability-failure-surface");
    fs::write(
        &workflow_path,
        r#"
workflow Solo {
  input task Task

  class Task {
    title string
  }

  failure error SoloFailure

  class SoloFailure {
    reason string
  }

  rule fail_now
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
            r#"{"task":{"title":"boom"}}"#,
            "--json",
            "dev",
            workflow_path.to_str().expect("utf-8 workflow path"),
            "--root",
            "Solo",
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
    let surface = status
        .get("failure_surface")
        .expect("failure_surface field");
    // The workflow failed on an author-declared `fail` terminal ...
    assert_eq!(
        surface.get("workflow_failed").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        surface.get("workflow_fail_kind").and_then(Value::as_str),
        Some("author")
    );
    assert_eq!(
        surface
            .get("workflow_fail_terminal")
            .and_then(Value::as_str),
        Some("error")
    );
    // ... with no provider/effect failure evidence recorded.
    assert_eq!(
        surface
            .get("provider_failure_present")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        surface
            .get("provider_failure_count")
            .and_then(Value::as_i64),
        Some(0)
    );

    let _ = fs::remove_file(store_path);
    let _ = fs::remove_file(workflow_path);
}
