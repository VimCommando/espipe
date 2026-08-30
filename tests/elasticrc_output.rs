use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
};

#[derive(Debug)]
struct RecordedRequest {
    headers: String,
    path: String,
}

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create temporary workspace");
    fs::write(dir.path().join("docs.ndjson"), "{\"message\":\"hello\"}\n")
        .expect("write input fixture");
    dir
}

fn write_config(dir: &tempfile::TempDir, filename: &str, contents: &str) -> std::path::PathBuf {
    let path = dir.path().join(filename);
    fs::write(&path, contents).expect("write Elastic CLI config");
    path
}

fn run_espipe(dir: &tempfile::TempDir, output_target: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_espipe"))
        .current_dir(dir.path())
        .env_remove("ELASTIC_CLI_CONFIG_FILE")
        .env("HOME", dir.path())
        .env_remove("USERPROFILE")
        .args(["docs.ndjson", output_target])
        .output()
        .expect("run espipe")
}

fn run_espipe_with_config(
    dir: &tempfile::TempDir,
    output_target: &str,
    config_path: &Path,
    authentication: &[&str],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_espipe"))
        .current_dir(dir.path())
        .env("ELASTIC_CLI_CONFIG_FILE", config_path)
        .env("HOME", dir.path())
        .env_remove("USERPROFILE")
        .env("LOG_LEVEL", "trace")
        .args(["docs.ndjson", output_target, "--uncompressed", "--quiet"])
        .args(authentication)
        .output()
        .expect("run espipe")
}

fn spawn_bulk_server() -> (String, Receiver<RecordedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let address = listener.local_addr().expect("read test server address");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept request");
        sender
            .send(handle_bulk_request(stream))
            .expect("record request");
    });
    (format!("http://{address}"), receiver)
}

fn handle_bulk_request(mut stream: TcpStream) -> RecordedRequest {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk).expect("read request");
        assert_ne!(count, 0, "connection closed before request headers");
        buffer.extend_from_slice(&chunk[..count]);
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index;
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    let body_start = header_end + 4;
    while buffer.len() < body_start + content_length {
        let count = stream.read(&mut chunk).expect("read request body");
        if count == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..count]);
    }
    let request_line = headers.lines().next().expect("request line");
    let path = request_line
        .split_whitespace()
        .nth(1)
        .expect("request path")
        .to_string();
    let response_body =
        r#"{"errors":false,"items":[{"index":{"_index":"logs-2026","_id":"1","status":201}}]}"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response_body}",
        response_body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("write response");
    RecordedRequest { headers, path }
}

#[test]
fn context_output_rejects_non_elasticsearch_application() {
    let dir = workspace();
    let output = run_espipe(&dir, ".production.kb:/logs-2026");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(
        stderr.contains("context outputs must select Elasticsearch"),
        "stderr: {stderr}"
    );
}

#[test]
fn context_output_rejects_unknown_app_and_malformed_index_forms() {
    let dir = workspace();

    for target in [
        ".production.search:/logs-2026",
        ".production.es:/",
        ".production.es:logs-2026",
        ".production.es://logs-2026",
    ] {
        let output = run_espipe(&dir, target);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(!output.status.success(), "target should fail: {target}");
        assert!(
            stderr.contains("Elastic CLI context"),
            "stderr for {target}: {stderr}"
        );
    }
}

#[test]
fn active_context_output_uses_resolved_url_index_and_api_key() {
    let dir = workspace();
    let (base_url, requests) = spawn_bulk_server();
    let config = format!(
        "current_context: production\ncontexts:\n  production:\n    elasticsearch:\n      url: \"{base_url}/elasticsearch/?ignored=true#fragment\"\n      auth:\n        api_key: context-key\n    kibana:\n      url: https://kibana.example\n      auth:\n        api_key: $(unknown:must-not-run)\n"
    );
    let config_path = write_config(&dir, "elasticrc.yml", &config);

    let output = run_espipe_with_config(&dir, ".es:/logs-2026", &config_path, &[]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = requests.recv().expect("receive bulk request");
    assert_eq!(request.path, "/elasticsearch/logs-2026/_bulk");
    assert!(
        request
            .headers
            .lines()
            .any(|line| line.eq_ignore_ascii_case("authorization: ApiKey context-key")),
        "headers: {}",
        request.headers
    );
    assert_eq!(
        fs::read_to_string(&config_path).expect("read Elastic CLI config"),
        config
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("context-key"));
}

#[test]
fn active_context_output_discovers_elasticrc_in_home() {
    let dir = workspace();
    let (base_url, requests) = spawn_bulk_server();
    write_config(
        &dir,
        ".elasticrc",
        &format!(
            "current_context: production\ncontexts:\n  production:\n    elasticsearch:\n      url: {base_url}\n"
        ),
    );

    let output = run_espipe(&dir, ".es:/logs-2026");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.recv().expect("receive bulk request").path,
        "/logs-2026/_bulk"
    );
}

#[test]
fn explicit_config_path_takes_precedence_over_home_discovery() {
    let dir = workspace();
    let (base_url, requests) = spawn_bulk_server();
    write_config(
        &dir,
        ".elasticrc",
        "current_context: home\ncontexts:\n  home:\n    elasticsearch:\n      url: http://127.0.0.1:1\n",
    );
    let explicit_config = write_config(
        &dir,
        "explicit.yml",
        &format!(
            "current_context: explicit\ncontexts:\n  explicit:\n    elasticsearch:\n      url: {base_url}\n"
        ),
    );

    let output = run_espipe_with_config(&dir, ".es:/logs-2026", &explicit_config, &[]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        requests.recv().expect("receive bulk request").path,
        "/logs-2026/_bulk"
    );
}

#[test]
fn context_output_reports_missing_config_context_and_service() {
    let dir = workspace();

    let missing_config = run_espipe(&dir, ".es:/logs-2026");
    let missing_config_stderr = String::from_utf8_lossy(&missing_config.stderr);
    assert!(!missing_config.status.success());
    assert!(
        missing_config_stderr.contains("Could not load Elastic CLI config")
            && missing_config_stderr.contains("no Elastic CLI config file found"),
        "stderr: {missing_config_stderr}"
    );

    let config_path = write_config(
        &dir,
        "elasticrc.yml",
        "current_context: production\ncontexts:\n  production:\n    kibana:\n      url: https://kibana.example\n",
    );

    let missing_context = run_espipe_with_config(&dir, ".missing.es:/logs-2026", &config_path, &[]);
    let missing_context_stderr = String::from_utf8_lossy(&missing_context.stderr);
    assert!(!missing_context.status.success());
    assert!(
        missing_context_stderr.contains("Elastic CLI context 'missing' was not found"),
        "stderr: {missing_context_stderr}"
    );

    let missing_service = run_espipe_with_config(&dir, ".es:/logs-2026", &config_path, &[]);
    let missing_service_stderr = String::from_utf8_lossy(&missing_service.stderr);
    assert!(!missing_service.status.success());
    assert!(
        missing_service_stderr.contains("does not define service 'elasticsearch'"),
        "stderr: {missing_service_stderr}"
    );
}

#[test]
fn context_output_reports_resolver_failure_without_secret_value() {
    let dir = workspace();
    let config_path = write_config(
        &dir,
        "elasticrc.yml",
        "current_context: production\ncontexts:\n  production:\n    elasticsearch:\n      url: https://elasticsearch.example\n      auth:\n        api_key: $(unknown:secret-value)\n",
    );

    let output = run_espipe_with_config(&dir, ".es:/logs-2026", &config_path, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(
        stderr.contains("unknown resolver 'unknown'"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("secret-value"), "stderr: {stderr}");
}

#[test]
fn dotted_named_context_output_uses_basic_authentication() {
    let dir = workspace();
    let (base_url, requests) = spawn_bulk_server();
    let config_path = write_config(
        &dir,
        "elasticrc.yml",
        &format!(
            "current_context: development\ncontexts:\n  development:\n    elasticsearch:\n      url: http://127.0.0.1:1\n  production.us-west:\n    elasticsearch:\n      url: {base_url}\n      auth:\n        username: elastic\n        password: context-password\n"
        ),
    );

    let output = run_espipe_with_config(
        &dir,
        ".production.us-west.elasticsearch:/logs-2026",
        &config_path,
        &[],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = requests.recv().expect("receive bulk request");
    assert_eq!(request.path, "/logs-2026/_bulk");
    assert!(
        request.headers.lines().any(|line| line
            .eq_ignore_ascii_case("authorization: Basic ZWxhc3RpYzpjb250ZXh0LXBhc3N3b3Jk")),
        "headers: {}",
        request.headers
    );
}

#[test]
fn explicit_authentication_overrides_context_authentication() {
    let dir = workspace();
    let (base_url, requests) = spawn_bulk_server();
    let config_path = write_config(
        &dir,
        "elasticrc.yml",
        &format!(
            "current_context: production\ncontexts:\n  production:\n    elasticsearch:\n      url: {base_url}\n      auth:\n        api_key: context-key\n"
        ),
    );

    let output = run_espipe_with_config(
        &dir,
        ".es:/logs-2026",
        &config_path,
        &["--apikey", "explicit-key"],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = requests.recv().expect("receive bulk request");
    assert!(
        request
            .headers
            .lines()
            .any(|line| line.eq_ignore_ascii_case("authorization: ApiKey explicit-key")),
        "headers: {}",
        request.headers
    );
    assert!(!request.headers.contains("context-key"));
}

#[test]
fn context_output_without_authentication_sends_no_authorization_header() {
    let dir = workspace();
    let (base_url, requests) = spawn_bulk_server();
    let config_path = write_config(
        &dir,
        "elasticrc.yml",
        &format!(
            "current_context: production\ncontexts:\n  production:\n    elasticsearch:\n      url: {base_url}\n"
        ),
    );

    let output = run_espipe_with_config(&dir, ".es:/logs-2026", &config_path, &[]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = requests.recv().expect("receive bulk request");
    assert!(
        !request
            .headers
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("authorization:")),
        "headers: {}",
        request.headers
    );
}
