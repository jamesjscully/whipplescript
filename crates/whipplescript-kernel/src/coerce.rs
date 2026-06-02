//! BAML-backed coerce effect contract.

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpStream,
    process::{Child, Command, Stdio},
    time::Duration,
};

use serde_json::{json, Value};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BamlServiceConfig {
    pub executable: String,
    pub args: Vec<String>,
    pub endpoint: String,
    pub env: BTreeMap<String, String>,
}

impl BamlServiceConfig {
    pub fn new(executable: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
            args: Vec::new(),
            endpoint: endpoint.into(),
            env: BTreeMap::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }
}

#[derive(Debug)]
pub struct ManagedBamlService {
    endpoint: String,
    child: Child,
}

impl ManagedBamlService {
    pub fn start(config: &BamlServiceConfig) -> std::io::Result<Self> {
        let mut command = Command::new(&config.executable);
        command.args(&config.args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        for (key, value) in &config.env {
            command.env(key, value);
        }
        let child = command.spawn()?;
        Ok(Self {
            endpoint: config.endpoint.clone(),
            child,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

impl Drop for ManagedBamlService {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BamlCoerceRequest {
    pub function_name: String,
    pub arguments_json: String,
    pub output_type: String,
    pub generated_baml_source_hash: String,
    pub input_schema_hash: String,
    pub output_schema_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BamlCoerceStatus {
    Succeeded,
    Failed,
    TimedOut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BamlCoerceResult {
    pub status: BamlCoerceStatus,
    pub value_json: Option<String>,
    pub error_json: Option<String>,
    pub summary: String,
    pub transcript: String,
    pub usage_json: String,
}

pub trait BamlClient {
    fn coerce(&self, request: &BamlCoerceRequest) -> BamlCoerceResult;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpBamlClient {
    endpoint: String,
    timeout: Duration,
    bearer_token: Option<String>,
}

impl HttpBamlClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            timeout: Duration::from_secs(30),
            bearer_token: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        let token = token.into();
        if !token.is_empty() {
            self.bearer_token = Some(token);
        }
        self
    }

    fn failed_result(
        &self,
        request: &BamlCoerceRequest,
        reason: impl Into<String>,
    ) -> BamlCoerceResult {
        BamlCoerceResult {
            status: BamlCoerceStatus::Failed,
            value_json: None,
            error_json: Some(
                json!({
                    "function_name": request.function_name,
                    "reason": reason.into(),
                    "endpoint": redacted_text_reference(&self.endpoint),
                    "recoverable": true,
                })
                .to_string(),
            ),
            summary: format!("BAML HTTP client failed for {}", request.function_name),
            transcript: format!(
                "POST {}/coerce function={}",
                self.endpoint.trim_end_matches('/'),
                request.function_name
            ),
            usage_json: "{}".to_owned(),
        }
    }

    fn decode_response(
        &self,
        request: &BamlCoerceRequest,
        status_code: u16,
        body: &str,
        transcript: String,
    ) -> BamlCoerceResult {
        let Ok(body_json) = serde_json::from_str::<Value>(body) else {
            return self.failed_result(
                request,
                format!("BAML endpoint returned non-JSON response with status {status_code}"),
            );
        };

        if !(200..300).contains(&status_code) {
            return BamlCoerceResult {
                status: BamlCoerceStatus::Failed,
                value_json: None,
                error_json: Some(
                    json!({
                        "function_name": request.function_name,
                        "status_code": status_code,
                        "body": body_json,
                        "recoverable": true,
                    })
                    .to_string(),
                ),
                summary: format!("BAML endpoint returned HTTP {status_code}"),
                transcript,
                usage_json: extract_json_field(&body_json, "usage")
                    .unwrap_or_else(|| "{}".to_owned()),
            };
        }

        let status = match body_json.get("status").and_then(Value::as_str) {
            Some(status) => BamlCoerceStatus::from_wire(status).unwrap_or(BamlCoerceStatus::Failed),
            None => BamlCoerceStatus::Succeeded,
        };
        let summary = body_json
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or(match status {
                BamlCoerceStatus::Succeeded => "coerce succeeded",
                BamlCoerceStatus::Failed => "coerce failed",
                BamlCoerceStatus::TimedOut => "coerce timed out",
            })
            .to_owned();
        let response_transcript = body_json
            .get("transcript")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or(transcript);
        BamlCoerceResult {
            status,
            value_json: extract_json_field(&body_json, "value"),
            error_json: extract_json_field(&body_json, "error"),
            summary,
            transcript: response_transcript,
            usage_json: extract_json_field(&body_json, "usage").unwrap_or_else(|| "{}".to_owned()),
        }
    }
}

impl BamlCoerceStatus {
    fn from_wire(value: &str) -> Option<Self> {
        match value {
            "succeeded" | "success" | "ok" => Some(Self::Succeeded),
            "failed" | "failure" | "error" => Some(Self::Failed),
            "timed_out" | "timeout" => Some(Self::TimedOut),
            _ => None,
        }
    }
}

impl BamlClient for HttpBamlClient {
    fn coerce(&self, request: &BamlCoerceRequest) -> BamlCoerceResult {
        let endpoint = match ParsedHttpEndpoint::parse(&self.endpoint) {
            Ok(endpoint) => endpoint,
            Err(error) => return self.failed_result(request, error),
        };
        let body = json!({
            "function_name": request.function_name,
            "arguments": json_from_str(&request.arguments_json),
            "output_type": request.output_type,
            "generated_baml_source_hash": request.generated_baml_source_hash,
            "input_schema_hash": request.input_schema_hash,
            "output_schema_hash": request.output_schema_hash,
        })
        .to_string();
        let path = endpoint.path_for("coerce");
        let transcript = format!("POST http://{}{}", endpoint.host_header, path);
        let authorization_header = self
            .bearer_token
            .as_ref()
            .map(|token| format!("Authorization: Bearer {token}\r\n"))
            .unwrap_or_default();
        let http_request = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\n{authorization_header}Content-Type: application/json\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: {length}\r\n\r\n{body}",
            host = endpoint.host_header,
            authorization_header = authorization_header,
            length = body.len(),
        );

        let mut stream = match TcpStream::connect(&endpoint.address) {
            Ok(stream) => stream,
            Err(error) => {
                return self.failed_result(
                    request,
                    format!("could not connect to BAML endpoint: {error}"),
                );
            }
        };
        let _ = stream.set_read_timeout(Some(self.timeout));
        let _ = stream.set_write_timeout(Some(self.timeout));
        if let Err(error) = stream.write_all(http_request.as_bytes()) {
            return self.failed_result(request, format!("could not write HTTP request: {error}"));
        }
        let mut response = String::new();
        if let Err(error) = stream.read_to_string(&mut response) {
            return self.failed_result(request, format!("could not read HTTP response: {error}"));
        }
        let Some((head, response_body)) = response.split_once("\r\n\r\n") else {
            return self.failed_result(request, "BAML endpoint returned malformed HTTP response");
        };
        let Some(status_code) = parse_status_code(head) else {
            return self.failed_result(request, "BAML endpoint returned missing HTTP status");
        };
        self.decode_response(request, status_code, response_body, transcript)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedHttpEndpoint {
    address: String,
    host_header: String,
    base_path: String,
}

impl ParsedHttpEndpoint {
    fn parse(endpoint: &str) -> Result<Self, String> {
        let Some(rest) = endpoint.strip_prefix("http://") else {
            return Err("BAML HTTP client currently supports only http:// endpoints".to_owned());
        };
        if endpoint.chars().any(|ch| ch.is_ascii_control()) {
            return Err("BAML endpoint contains invalid control characters".to_owned());
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.is_empty() {
            return Err("BAML endpoint is missing a host".to_owned());
        }
        if !valid_authority(authority) {
            return Err("BAML endpoint contains an invalid host or port".to_owned());
        }
        if path.chars().any(|ch| ch.is_ascii_whitespace()) || path.contains(['?', '#']) {
            return Err("BAML endpoint contains an invalid path".to_owned());
        }
        let address = if authority.contains(':') {
            authority.to_owned()
        } else {
            format!("{authority}:80")
        };
        Ok(Self {
            address,
            host_header: authority.to_owned(),
            base_path: path.trim_matches('/').to_owned(),
        })
    }

    fn path_for(&self, suffix: &str) -> String {
        if self.base_path.is_empty() {
            format!("/{suffix}")
        } else {
            format!("/{}/{}", self.base_path, suffix)
        }
    }
}

fn valid_authority(authority: &str) -> bool {
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    !host.is_empty()
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        && port.is_none_or(|port| !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()))
}

fn parse_status_code(head: &str) -> Option<u16> {
    head.lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse::<u16>()
        .ok()
}

fn extract_json_field(body: &Value, field: &str) -> Option<String> {
    body.get(field).map(Value::to_string)
}

fn redacted_text_reference(text: &str) -> String {
    format!(
        "<redacted bytes={} chars={}>",
        text.len(),
        text.chars().count()
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeBamlClient {
    result: BamlCoerceResult,
}

impl FakeBamlClient {
    pub fn succeeds(value_json: impl Into<String>) -> Self {
        Self {
            result: BamlCoerceResult {
                status: BamlCoerceStatus::Succeeded,
                value_json: Some(value_json.into()),
                error_json: None,
                summary: "coerce succeeded".to_owned(),
                transcript: "fake baml transcript\n".to_owned(),
                usage_json: r#"{"input_tokens":1,"output_tokens":1}"#.to_owned(),
            },
        }
    }

    pub fn fails(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            result: BamlCoerceResult {
                status: BamlCoerceStatus::Failed,
                value_json: None,
                error_json: Some(
                    json!({
                        "reason": reason,
                        "recoverable": true,
                    })
                    .to_string(),
                ),
                summary: "coerce failed".to_owned(),
                transcript: "fake baml failure transcript\n".to_owned(),
                usage_json: r#"{"input_tokens":1,"output_tokens":0}"#.to_owned(),
            },
        }
    }

    pub fn times_out(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            result: BamlCoerceResult {
                status: BamlCoerceStatus::TimedOut,
                value_json: None,
                error_json: Some(
                    json!({
                        "reason": reason,
                        "recoverable": true,
                    })
                    .to_string(),
                ),
                summary: "coerce timed out".to_owned(),
                transcript: "fake baml timeout transcript\n".to_owned(),
                usage_json: r#"{"input_tokens":1,"output_tokens":0}"#.to_owned(),
            },
        }
    }
}

impl BamlClient for FakeBamlClient {
    fn coerce(&self, request: &BamlCoerceRequest) -> BamlCoerceResult {
        let mut result = self.result.clone();
        let request_json = json!({
            "function_name": request.function_name,
            "arguments": json_from_str(&request.arguments_json),
            "output_type": request.output_type,
        });
        result.transcript.push_str(&request_json.to_string());
        result
    }
}

fn json_from_str(source: &str) -> Value {
    serde_json::from_str(source).unwrap_or_else(|_| Value::String(source.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn fake_baml_client_returns_typed_value() {
        let client = FakeBamlClient::succeeds(r#"{"status":"Accept"}"#);
        let result = client.coerce(&BamlCoerceRequest {
            function_name: "reviewWork".to_owned(),
            arguments_json: r#"{"summary":"done"}"#.to_owned(),
            output_type: "WorkReview".to_owned(),
            generated_baml_source_hash: "baml".to_owned(),
            input_schema_hash: "input".to_owned(),
            output_schema_hash: "output".to_owned(),
        });

        assert_eq!(result.status, BamlCoerceStatus::Succeeded);
        assert_eq!(result.value_json.as_deref(), Some(r#"{"status":"Accept"}"#));
        assert!(result.transcript.contains("reviewWork"));
    }

    #[test]
    fn managed_service_exposes_endpoint() {
        let service = ManagedBamlService::start(
            &BamlServiceConfig::new("sh", "http://127.0.0.1:0")
                .arg("-c")
                .arg("sleep 1"),
        )
        .expect("service starts");

        assert_eq!(service.endpoint(), "http://127.0.0.1:0");
    }

    #[test]
    fn http_client_posts_coerce_request_and_decodes_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binds test listener");
        let endpoint = format!("http://{}", listener.local_addr().expect("has address"));
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accepts client");
            let mut buffer = [0_u8; 2048];
            let count = stream.read(&mut buffer).expect("reads request");
            let request = String::from_utf8_lossy(&buffer[..count]);
            assert!(request.starts_with("POST /coerce HTTP/1.1"));
            assert!(request.contains("Authorization: Bearer test-token-123456\r\n"));
            assert!(request.contains(r#""function_name":"reviewWork""#));
            assert!(request.contains(r#""arguments":{"summary":"done"}"#));

            let body = json!({
                "status": "succeeded",
                "value": {"decision": "accept"},
                "summary": "accepted",
                "transcript": "baml transcript",
                "usage": {"input_tokens": 3, "output_tokens": 2},
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("writes response");
        });

        let client = HttpBamlClient::new(endpoint)
            .with_timeout(Duration::from_secs(1))
            .with_bearer_token("test-token-123456");
        let result = client.coerce(&BamlCoerceRequest {
            function_name: "reviewWork".to_owned(),
            arguments_json: r#"{"summary":"done"}"#.to_owned(),
            output_type: "WorkReview".to_owned(),
            generated_baml_source_hash: "baml".to_owned(),
            input_schema_hash: "input".to_owned(),
            output_schema_hash: "output".to_owned(),
        });

        server.join().expect("server exits");
        assert_eq!(result.status, BamlCoerceStatus::Succeeded);
        assert_eq!(
            result.value_json.as_deref(),
            Some(r#"{"decision":"accept"}"#)
        );
        assert_eq!(result.summary, "accepted");
        assert_eq!(result.transcript, "baml transcript");
        assert_eq!(result.usage_json, r#"{"input_tokens":3,"output_tokens":2}"#);
    }

    #[test]
    fn http_client_rejects_endpoint_control_characters_without_echoing_endpoint() {
        let endpoint = "http://127.0.0.1:1/base\r\nX-Injected: true";
        let client = HttpBamlClient::new(endpoint).with_timeout(Duration::from_millis(10));
        let result = client.coerce(&BamlCoerceRequest {
            function_name: "reviewWork".to_owned(),
            arguments_json: r#"{"summary":"done"}"#.to_owned(),
            output_type: "WorkReview".to_owned(),
            generated_baml_source_hash: "baml".to_owned(),
            input_schema_hash: "input".to_owned(),
            output_schema_hash: "output".to_owned(),
        });

        assert_eq!(result.status, BamlCoerceStatus::Failed);
        let error = result.error_json.expect("error json");
        assert!(error.contains("\"endpoint\":\"<redacted"));
        assert!(!error.contains("X-Injected"));
    }

    #[test]
    fn real_baml_coerce_endpoint_smoke() {
        let Ok(endpoint) = std::env::var("WHIPPLESCRIPT_BAML_TEST_ENDPOINT") else {
            return;
        };
        let Ok(function_name) = std::env::var("WHIPPLESCRIPT_BAML_TEST_FUNCTION") else {
            return;
        };
        let Ok(arguments_json) = std::env::var("WHIPPLESCRIPT_BAML_TEST_ARGUMENTS_JSON") else {
            return;
        };
        let Ok(output_type) = std::env::var("WHIPPLESCRIPT_BAML_TEST_OUTPUT_TYPE") else {
            return;
        };

        let mut client = HttpBamlClient::new(endpoint).with_timeout(Duration::from_secs(10));
        if let Ok(token) = std::env::var("WHIPPLESCRIPT_BAML_AUTH_TOKEN") {
            client = client.with_bearer_token(token);
        }
        let result = client.coerce(&BamlCoerceRequest {
            function_name,
            arguments_json,
            output_type,
            generated_baml_source_hash: "real-baml-smoke".to_owned(),
            input_schema_hash: "real-baml-smoke-input".to_owned(),
            output_schema_hash: "real-baml-smoke-output".to_owned(),
        });

        assert_eq!(
            result.status,
            BamlCoerceStatus::Succeeded,
            "real BAML coerce smoke failed: status={:?} summary={}",
            result.status,
            result.summary,
        );
        assert!(
            result.value_json.is_some(),
            "real BAML coerce smoke returned no typed value"
        );
    }
}
