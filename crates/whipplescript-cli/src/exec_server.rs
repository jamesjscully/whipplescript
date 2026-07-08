//! Class-A executor sidecar (compute plane P8): a stateless HTTP server that
//! runs sha-pinned scripts on behalf of a workflow host that cannot spawn
//! processes (the DO isolate raises `NeedsHttp`; its shell fetches here).
//!
//! v1 protocol (`whip-executor/1`, one request-response per exec — §4 of
//! spec/compute-plane-design-note.md): the request carries the script bytes
//! inline (verified against the pinned sha256 before running — same TOCTOU
//! discipline as native script capabilities), the argv with a script-slot
//! index, resolved env values, and the JSON stdin. Manifest-ref + pull-
//! missing-blobs materialization joins when the object tier lands.
//!
//! Hermeticity is enforced harder than native exec: the child gets a CLEANED
//! environment (only the declared env plus PATH) — the sidecar is stronger
//! than native, per the design note's IFC-span section. Network egress denial
//! is a container property, not enforced here.
//!
//! Server style matches the repo's execution model: hand-rolled HTTP/1.1 over
//! `TcpListener`, thread per connection, threads not async.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub use whipplescript_kernel::exec_http::EXECUTOR_PROTOCOL;
use whipplescript_kernel::exec_http::{base64_decode, sha256_hex};

/// Per-stream response cap. Bounded so a runaway script cannot balloon the
/// response; the flag tells the caller truncation happened.
const STREAM_CAP_BYTES: usize = 512 * 1024;

/// Default and ceiling for the per-exec timeout.
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_TIMEOUT_MS: u64 = 300_000;

/// Serve forever on `bind` (e.g. `127.0.0.1:8080`).
pub fn serve(bind: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(bind)?;
    serve_on(listener)
}

/// Serve forever on an already-bound listener (tests bind `:0` first).
pub fn serve_on(listener: TcpListener) -> std::io::Result<()> {
    eprintln!(
        "whip executor listening on {} ({EXECUTOR_PROTOCOL})",
        listener.local_addr()?
    );
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                std::thread::spawn(move || {
                    let _ = handle_connection(stream);
                });
            }
            Err(error) => eprintln!("executor: accept failed: {error}"),
        }
    }
    Ok(())
}

fn handle_connection(stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();

    let mut content_length = 0usize;
    let mut websocket_key = None;
    let mut wants_upgrade = false;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            } else if name.eq_ignore_ascii_case("sec-websocket-key") {
                websocket_key = Some(value.trim().to_owned());
            } else if name.eq_ignore_ascii_case("upgrade")
                && value.trim().eq_ignore_ascii_case("websocket")
            {
                wants_upgrade = true;
            }
        }
    }

    // Class-B turn channel (whip-turn/1): hand the raw socket to the
    // WebSocket handler. Safe because an upgrade request has no body and the
    // client sends no frames until it sees the 101 — the buffered reader has
    // consumed exactly through the header terminator.
    if method == "GET" && path == "/turn" && wants_upgrade {
        if let Some(key) = websocket_key {
            drop(reader);
            return crate::turn_server::handle_turn_websocket(stream, &key);
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    let (status, response_body) = match (method.as_str(), path.as_str()) {
        ("GET", "/healthz") => (200, json!({"protocol": EXECUTOR_PROTOCOL, "ok": true})),
        ("POST", "/exec") => match serde_json::from_slice::<Value>(&body) {
            Ok(request) => match handle_exec_request(&request) {
                Ok(response) => (200, response),
                Err((status, message)) => (status, json!({"error": message})),
            },
            Err(error) => (400, json!({"error": format!("invalid JSON body: {error}")})),
        },
        // Class-B blocking form: run (or re-attach to) a whole agent turn and
        // answer with its final outcome. The WS form on GET /turn streams.
        ("POST", "/turn") => match serde_json::from_slice::<Value>(&body) {
            Ok(request) => match crate::turn_server::handle_turn_http(&request) {
                Ok(response) => (200, response),
                Err((status, message)) => (status, json!({"error": message})),
            },
            Err(error) => (400, json!({"error": format!("invalid JSON body: {error}")})),
        },
        _ => (
            404,
            json!({"error": "unknown route; POST /exec or GET /healthz"}),
        ),
    };

    let payload = response_body.to_string();
    let mut stream = stream;
    write!(
        stream,
        "HTTP/1.1 {status} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
        match status {
            200 => "OK",
            400 => "Bad Request",
            404 => "Not Found",
            _ => "Internal Server Error",
        },
        payload.len(),
    )?;
    stream.flush()
}

/// Validate + run one exec request. Pure with respect to the transport, so
/// it is testable without sockets.
pub fn handle_exec_request(request: &Value) -> Result<Value, (u16, String)> {
    let protocol = request
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if protocol != EXECUTOR_PROTOCOL {
        return Err((
            400,
            format!("unsupported protocol `{protocol}`; expected `{EXECUTOR_PROTOCOL}`"),
        ));
    }
    let effect_id = request
        .get("effect_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let expected_sha = request
        .get("script_sha256")
        .and_then(Value::as_str)
        .ok_or((400, "script_sha256 is required".to_owned()))?
        .to_ascii_lowercase();
    let script_b64 = request
        .get("script_b64")
        .and_then(Value::as_str)
        .ok_or((400, "script_b64 is required".to_owned()))?;
    let script_bytes =
        base64_decode(script_b64).ok_or((400, "script_b64 is not valid base64".to_owned()))?;
    let actual_sha = sha256_hex(&script_bytes);
    if actual_sha != expected_sha {
        return Err((
            400,
            format!("script hash mismatch: expected {expected_sha}, got {actual_sha}"),
        ));
    }
    let argv = request
        .get("argv")
        .and_then(Value::as_array)
        .ok_or((400, "argv array is required".to_owned()))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or((400, "argv values must be strings".to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if argv.is_empty() {
        return Err((400, "argv must not be empty".to_owned()));
    }
    let script_index = request
        .get("script_index")
        .and_then(Value::as_u64)
        .ok_or((400, "script_index is required".to_owned()))? as usize;
    if script_index >= argv.len() {
        return Err((400, "script_index is out of range".to_owned()));
    }
    let env = match request.get("env") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(entries)) => entries
            .iter()
            .map(|(name, value)| {
                value
                    .as_str()
                    .map(|value| (name.clone(), value.to_owned()))
                    .ok_or((400, "env values must be strings".to_owned()))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err((400, "env must be an object".to_owned())),
    };
    let stdin_json = request
        .get("stdin")
        .cloned()
        .unwrap_or(Value::Null)
        .to_string();
    let script_ext = request
        .get("script_ext")
        .and_then(Value::as_str)
        .unwrap_or("");
    let timeout_ms = request
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .min(MAX_TIMEOUT_MS);

    let staged = stage_verified_script(&actual_sha, &script_bytes, script_ext)
        .map_err(|error| (500, format!("failed to stage script: {error}")))?;
    let mut argv = argv;
    argv[script_index] = staged.display().to_string();

    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    // Cleaned environment: only the declared values plus PATH. The sidecar is
    // deliberately stronger than native exec here.
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    for (name, value) in &env {
        command.env(name, value);
    }
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let outcome = run_with_timeout(command, &stdin_json, Duration::from_millis(timeout_ms));
    let _ = std::fs::remove_file(&staged);
    let (exit_code, timed_out, stdout, stderr) =
        outcome.map_err(|error| (500, format!("exec failed: {error}")))?;

    let (stdout, stdout_truncated) = cap_stream(stdout);
    let (stderr, stderr_truncated) = cap_stream(stderr);
    Ok(json!({
        "protocol": EXECUTOR_PROTOCOL,
        "effect_id": effect_id,
        "exit_code": exit_code,
        "timed_out": timed_out,
        "stdout": stdout,
        "stdout_truncated": stdout_truncated,
        "stderr": stderr,
        "stderr_truncated": stderr_truncated,
    }))
}

/// Spawn, hand off stdin (EPIPE-tolerant: a script that never reads stdin is
/// normal), and wait with a kill-on-timeout loop. Returns
/// `(exit_code, timed_out, stdout, stderr)`.
fn run_with_timeout(
    mut command: Command,
    stdin_json: &str,
    timeout: Duration,
) -> Result<(i64, bool, String, String), String> {
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn: {error}"))?;
    if let Some(stdin) = child.stdin.as_mut() {
        if let Err(error) = stdin.write_all(stdin_json.as_bytes()) {
            if error.kind() != std::io::ErrorKind::BrokenPipe {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to write stdin: {error}"));
            }
        }
    }
    drop(child.stdin.take());

    // Drain pipes on threads so a chatty child cannot deadlock on a full pipe
    // while we poll for exit.
    let stdout_handle = child.stdout.take().map(spawn_drain);
    let stderr_handle = child.stderr.take().map(spawn_drain);

    let start = Instant::now();
    let (exit_code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (i64::from(status.code().unwrap_or(-1)), false),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break (-1, true);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("failed to wait: {error}")),
        }
    };
    if timed_out {
        // Grandchildren of the killed process may still hold the pipes (kill
        // reaches the direct child only); joining the drain threads would
        // block until they exit. Abandon the drains — the threads end when
        // the pipes close — and report empty streams for the killed run.
        return Ok((exit_code, timed_out, String::new(), String::new()));
    }
    let stdout = stdout_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr_handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    Ok((exit_code, timed_out, stdout, stderr))
}

fn spawn_drain<R: Read + Send + 'static>(mut source: R) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = source.read_to_end(&mut buffer);
        String::from_utf8_lossy(&buffer).into_owned()
    })
}

fn cap_stream(stream: String) -> (String, bool) {
    if stream.len() <= STREAM_CAP_BYTES {
        return (stream, false);
    }
    let mut end = STREAM_CAP_BYTES;
    while !stream.is_char_boundary(end) {
        end -= 1;
    }
    (stream[..end].to_owned(), true)
}

/// Stage the verified bytes under a content-addressed temp path and make the
/// file executable (argv may invoke it directly).
fn stage_verified_script(sha256: &str, bytes: &[u8], extension: &str) -> std::io::Result<PathBuf> {
    let suffix = if extension.is_empty() {
        String::new()
    } else {
        format!(".{extension}")
    };
    let path = std::env::temp_dir().join(format!("whip-executor-{sha256}{suffix}"));
    std::fs::write(&path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use whipplescript_kernel::exec_http::base64_encode;

    fn exec_request(script: &str, stdin: Value) -> Value {
        json!({
            "protocol": EXECUTOR_PROTOCOL,
            "effect_id": "effect-1",
            "script_sha256": sha256_hex(script.as_bytes()),
            "script_b64": base64_encode(script.as_bytes()),
            "script_ext": "sh",
            "argv": ["sh", "{script}"],
            "script_index": 1,
            "stdin": stdin,
        })
    }

    #[test]
    fn base64_roundtrip() {
        for sample in [
            &b""[..],
            &b"a"[..],
            &b"ab"[..],
            &b"abc"[..],
            &b"echo hello # \xff\x00 binary"[..],
        ] {
            let encoded = base64_encode(sample);
            assert_eq!(base64_decode(&encoded).expect("decodes"), sample);
        }
        assert!(base64_decode("not!!base64").is_none());
    }

    #[test]
    fn exec_request_runs_script_with_stdin_and_env() {
        let script = "read line\necho \"got:$line:$JUDGE_MODE\"\necho oops >&2\nexit 3\n";
        let mut request = exec_request(script, json!({"n": 1}));
        request["env"] = json!({"JUDGE_MODE": "strict"});
        let response = handle_exec_request(&request).expect("executes");
        assert_eq!(response["exit_code"], json!(3));
        assert_eq!(response["timed_out"], json!(false));
        assert_eq!(response["stdout"], json!("got:{\"n\":1}:strict\n"));
        assert_eq!(response["stderr"], json!("oops\n"));
        assert_eq!(response["effect_id"], json!("effect-1"));
    }

    #[test]
    fn exec_request_cleans_the_environment() {
        // A host env var not declared in the request must not leak through.
        std::env::set_var("WHIP_EXECUTOR_LEAK_PROBE", "leaked");
        let response = handle_exec_request(&exec_request(
            "echo \"probe:${WHIP_EXECUTOR_LEAK_PROBE:-clean}\"\n",
            Value::Null,
        ))
        .expect("executes");
        assert_eq!(response["stdout"], json!("probe:clean\n"));
    }

    #[test]
    fn exec_request_rejects_hash_mismatch_and_bad_shapes() {
        let mut tampered = exec_request("echo hi\n", Value::Null);
        tampered["script_b64"] = json!(base64_encode(b"echo tampered\n"));
        let (status, message) = handle_exec_request(&tampered).expect_err("hash mismatch");
        assert_eq!(status, 400);
        assert!(message.contains("hash mismatch"), "{message}");

        let mut wrong_protocol = exec_request("echo hi\n", Value::Null);
        wrong_protocol["protocol"] = json!("bogus/9");
        let (status, message) =
            handle_exec_request(&wrong_protocol).expect_err("protocol rejected");
        assert_eq!(status, 400);
        assert!(message.contains("unsupported protocol"), "{message}");

        let mut bad_index = exec_request("echo hi\n", Value::Null);
        bad_index["script_index"] = json!(9);
        let (status, message) = handle_exec_request(&bad_index).expect_err("index rejected");
        assert_eq!(status, 400);
        assert!(message.contains("out of range"), "{message}");
    }

    #[test]
    fn exec_request_kills_on_timeout() {
        let mut request = exec_request("sleep 5\n", Value::Null);
        request["timeout_ms"] = json!(150);
        let started = Instant::now();
        let response = handle_exec_request(&request).expect("timeout is a response");
        assert!(started.elapsed() < Duration::from_secs(4), "killed early");
        assert_eq!(response["timed_out"], json!(true));
        assert_eq!(response["exit_code"], json!(-1));
    }

    #[test]
    fn server_answers_exec_and_health_over_http() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        let address = listener.local_addr().expect("local addr");
        std::thread::spawn(move || {
            let _ = serve_on(listener);
        });

        let health: Value = ureq::get(&format!("http://{address}/healthz"))
            .call()
            .expect("healthz")
            .into_json()
            .expect("health json");
        assert_eq!(health["ok"], json!(true));

        let response: Value = ureq::post(&format!("http://{address}/exec"))
            .send_json(exec_request("echo over-http\n", Value::Null))
            .expect("exec call")
            .into_json()
            .expect("exec json");
        assert_eq!(response["exit_code"], json!(0));
        assert_eq!(response["stdout"], json!("over-http\n"));

        let error = ureq::post(&format!("http://{address}/exec"))
            .send_json(json!({"protocol": "bogus"}))
            .expect_err("bad request errors");
        match error {
            ureq::Error::Status(status, _) => assert_eq!(status, 400),
            other => panic!("unexpected transport error: {other}"),
        }
    }
}
