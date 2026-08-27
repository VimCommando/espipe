use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Command, Output},
    sync::{Arc, Mutex},
    thread,
};

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    body: String,
}

#[derive(Debug)]
struct MockResponse {
    status: u16,
    body: String,
}

fn response(status: u16, body: Value) -> MockResponse {
    MockResponse {
        status,
        body: body.to_string(),
    }
}

fn spawn_server(responses: Vec<MockResponse>) -> (String, Arc<Mutex<Vec<RecordedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_requests = Arc::clone(&requests);
    let responses = Arc::new(Mutex::new(VecDeque::from(responses)));

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                break;
            };
            handle_connection(stream, Arc::clone(&thread_requests), Arc::clone(&responses));
        }
    });

    (format!("http://{address}"), requests)
}

fn handle_connection(
    mut stream: TcpStream,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    responses: Arc<Mutex<VecDeque<MockResponse>>>,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk).unwrap();
        if count == 0 {
            return;
        }
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
        let count = stream.read(&mut chunk).unwrap();
        if count == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..count]);
    }
    let request_line = headers.lines().next().unwrap();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap().to_string();
    let path = request_parts.next().unwrap().to_string();
    let body =
        String::from_utf8_lossy(&buffer[body_start..body_start + content_length]).to_string();
    requests.lock().unwrap().push(RecordedRequest {
        method,
        path: path.clone(),
        body: body.clone(),
    });

    let response = if path.contains("/_bulk") {
        let item_count = body.lines().count() / 2;
        let items = (0..item_count)
            .map(|_| json!({"index":{"_index":"knowledge","_id":"1","status":201}}))
            .collect::<Vec<_>>();
        MockResponse {
            status: 200,
            body: json!({"errors": false, "items": items}).to_string(),
        }
    } else {
        responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| MockResponse {
                status: 500,
                body: json!({"error": "unexpected request"}).to_string(),
            })
    };
    let reason = match response.status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Internal Server Error",
    };
    let wire = format!(
        "HTTP/1.1 {} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        response.status,
        response.body.len(),
        response.body
    );
    stream.write_all(wire.as_bytes()).unwrap();
}

fn run_espipe(current_dir: &Path, input: &Path, output: &str, extra: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_espipe"));
    command
        .current_dir(current_dir)
        .arg(input)
        .arg(output)
        .arg("--template")
        .arg("_okf")
        .arg("--uncompressed")
        .args(extra);
    command.output().expect("run espipe")
}

fn input_file(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("input.ndjson");
    fs::write(&path, "{\"message\":\"hello\"}\n").unwrap();
    path
}

#[test]
fn bundled_okf_creates_default_template_outside_source_tree() {
    let directory = tempfile::tempdir().unwrap();
    let input = input_file(directory.path());
    let (base_url, requests) = spawn_server(vec![
        response(404, json!({"status": 404})),
        response(200, json!({"acknowledged": true})),
    ]);

    let output = run_espipe(
        directory.path(),
        &input,
        &format!("{base_url}/team-knowledge"),
        &[],
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/_index_template/open-knowledge-format");
    assert_eq!(requests[1].method, "PUT");
    assert_eq!(requests[1].path, "/_index_template/open-knowledge-format");
    let body: Value = serde_json::from_str(&requests[1].body).unwrap();
    assert_eq!(body["index_patterns"], json!(["team-knowledge"]));
    assert_eq!(body["_meta"]["okf_version"], "0.2");
    assert_eq!(body["template"]["mappings"]["date_detection"], false);
    assert_eq!(requests[2].path, "/team-knowledge/_bulk");
}

#[test]
fn bundled_okf_override_uses_only_the_selected_name_and_skips_exact_target() {
    let directory = tempfile::tempdir().unwrap();
    let input = input_file(directory.path());
    let stored = json!({
        "index_templates": [{
            "name": "team-okf",
            "index_template": {"index_patterns": ["team-knowledge"]}
        }]
    });
    let (base_url, requests) = spawn_server(vec![response(200, stored)]);

    let output = run_espipe(
        directory.path(),
        &input,
        &format!("{base_url}/team-knowledge"),
        &["--template-name", "team-okf"],
    );

    assert!(output.status.success());
    let requests = requests.lock().unwrap();
    assert_eq!(requests[0].path, "/_index_template/team-okf");
    assert!(
        requests
            .iter()
            .all(|request| !request.path.contains("open-knowledge-format"))
    );
    assert!(requests.iter().all(|request| request.method != "PUT"));
    assert_eq!(requests[1].path, "/team-knowledge/_bulk");
}

#[test]
fn bundled_okf_appends_exact_target_and_preserves_stored_fields() {
    let directory = tempfile::tempdir().unwrap();
    let input = input_file(directory.path());
    let stored_body = json!({
        "index_patterns": ["team-*"],
        "composed_of": ["cluster-component"],
        "priority": 42,
        "version": 9,
        "_meta": {"owner": "search-team"},
        "template": {
            "settings": {"number_of_shards": 3},
            "mappings": {"properties": {"cluster_only": {"type": "long"}}},
            "aliases": {"knowledge-read": {}}
        }
    });
    let stored = json!({
        "index_templates": [{
            "name": "open-knowledge-format",
            "index_template": stored_body
        }]
    });
    let (base_url, requests) = spawn_server(vec![
        response(200, stored),
        response(200, json!({"acknowledged": true})),
    ]);

    let output = run_espipe(
        directory.path(),
        &input,
        &format!("{base_url}/team-knowledge"),
        &[],
    );

    assert!(output.status.success());
    let requests = requests.lock().unwrap();
    let updated: Value = serde_json::from_str(&requests[1].body).unwrap();
    assert_eq!(
        updated["index_patterns"],
        json!(["team-*", "team-knowledge"])
    );
    assert_eq!(updated["composed_of"], json!(["cluster-component"]));
    assert_eq!(updated["priority"], 42);
    assert_eq!(updated["version"], 9);
    assert_eq!(updated["_meta"]["owner"], "search-team");
    assert_eq!(updated["template"]["settings"]["number_of_shards"], 3);
    assert_eq!(
        updated["template"]["mappings"]["properties"]["cluster_only"]["type"],
        "long"
    );
    assert!(
        updated["template"]["aliases"]
            .get("knowledge-read")
            .is_some()
    );
}

#[test]
fn bundled_okf_create_only_handles_missing_existing_and_new_target_branches() {
    let directory = tempfile::tempdir().unwrap();
    let input = input_file(directory.path());
    let (base_url, requests) = spawn_server(vec![
        response(404, json!({"status": 404})),
        response(200, json!({"acknowledged": true})),
    ]);
    let output = run_espipe(
        directory.path(),
        &input,
        &format!("{base_url}/team-knowledge"),
        &["--template-overwrite=false"],
    );
    assert!(output.status.success());
    assert_eq!(
        requests.lock().unwrap()[1].path,
        "/_index_template/open-knowledge-format?create=true"
    );

    let existing = json!({
        "index_templates": [{
            "name": "open-knowledge-format",
            "index_template": {"index_patterns": ["other-knowledge"]}
        }]
    });
    let (base_url, requests) = spawn_server(vec![response(200, existing)]);
    let output = run_espipe(
        directory.path(),
        &input,
        &format!("{base_url}/team-knowledge"),
        &["--template-overwrite=false"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--template-overwrite=true"));
    assert_eq!(requests.lock().unwrap().len(), 1);

    let existing = json!({
        "index_templates": [{
            "name": "open-knowledge-format",
            "index_template": {"index_patterns": ["team-knowledge"]}
        }]
    });
    let (base_url, requests) = spawn_server(vec![response(200, existing)]);
    let output = run_espipe(
        directory.path(),
        &input,
        &format!("{base_url}/team-knowledge"),
        &["--template-overwrite=false"],
    );
    assert!(output.status.success());
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[1].path, "/team-knowledge/_bulk");
    assert!(
        requests[1..]
            .iter()
            .all(|request| !request.path.contains("_index_template"))
    );
}

#[test]
fn bundled_okf_rejects_lookup_failures_and_malformed_stored_templates() {
    let directory = tempfile::tempdir().unwrap();
    let input = input_file(directory.path());
    let (base_url, requests) = spawn_server(vec![response(
        401,
        json!({"error": {"type": "security_exception"}}),
    )]);
    let output = run_espipe(
        directory.path(),
        &input,
        &format!("{base_url}/team-knowledge"),
        &[],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to look up index template"));
    assert_eq!(requests.lock().unwrap().len(), 1);

    let malformed = json!({
        "index_templates": [{
            "name": "open-knowledge-format",
            "index_template": {"index_patterns": "team-*"}
        }]
    });
    let (base_url, requests) = spawn_server(vec![response(200, malformed)]);
    let output = run_espipe(
        directory.path(),
        &input,
        &format!("{base_url}/team-knowledge"),
        &[],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("index_patterns must be an array"));
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[test]
fn bundled_preflight_runs_before_input_construction() {
    let directory = tempfile::tempdir().unwrap();
    let input = input_file(directory.path());
    let (base_url, requests) = spawn_server(vec![response(
        401,
        json!({"error": {"type": "security_exception"}}),
    )]);

    let output = run_espipe(
        directory.path(),
        &input,
        &format!("{base_url}/team-knowledge"),
        &["--content", "invalid.field"],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to look up index template"));
    assert!(!stderr.contains("--content value"));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
}

#[test]
fn unknown_bundled_selector_fails_before_input_access() {
    let directory = tempfile::tempdir().unwrap();
    let missing_input = directory.path().join("missing.ndjson");
    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .current_dir(directory.path())
        .arg(&missing_input)
        .arg("http://127.0.0.1:9/team-knowledge")
        .arg("--template")
        .arg("_missing")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown bundled template '_missing'"));
    assert!(stderr.contains("_okf"));
    assert!(!stderr.contains("failed to open"));
}

#[test]
fn underscore_inside_file_template_path_keeps_file_behavior() {
    let directory = tempfile::tempdir().unwrap();
    let input = input_file(directory.path());
    let templates = directory.path().join("templates");
    fs::create_dir(&templates).unwrap();
    fs::write(
        templates.join("_okf.json"),
        r#"{"index_patterns":["team-*"]}"#,
    )
    .unwrap();
    let (base_url, requests) = spawn_server(vec![response(200, json!({"acknowledged": true}))]);
    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .current_dir(directory.path())
        .arg(&input)
        .arg(format!("{base_url}/team-knowledge"))
        .arg("--template")
        .arg("templates/_okf.json")
        .arg("--uncompressed")
        .output()
        .unwrap();

    assert!(output.status.success());
    let requests = requests.lock().unwrap();
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(requests[0].path, "/_index_template/_okf");
    assert!(requests.iter().all(|request| request.method != "GET"));
}
