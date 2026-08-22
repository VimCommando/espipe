use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Command, Output},
    sync::{Arc, Mutex},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    content_type: Option<String>,
    body: String,
}

fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn temp_workspace_dir(prefix: &str) -> tempfile::TempDir {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    fs::create_dir_all(&target_dir).expect("create target directory");
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(target_dir)
        .expect("create workspace temp dir")
}

fn write_input_file(dir: &PathBuf) -> PathBuf {
    let path = dir.join("input.ndjson");
    fs::write(&path, "{\"message\":\"hello\"}\n{\"message\":\"world\"}\n").unwrap();
    path
}

fn write_template_file(dir: &PathBuf, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

fn write_pipeline_file(dir: &PathBuf, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn run_espipe(args: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_espipe"))
        .args(args)
        .output()
        .expect("run espipe")
}

fn spawn_server(template_status: u16) -> (String, Arc<Mutex<Vec<RecordedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_requests = Arc::clone(&requests);

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                break;
            };
            let requests = Arc::clone(&thread_requests);
            thread::spawn(move || handle_connection(stream, template_status, requests));
        }
    });

    (format!("http://{addr}"), requests)
}

fn handle_connection(
    mut stream: TcpStream,
    template_status: u16,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end;
    loop {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            return;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_header_end(&buffer) {
            header_end = index;
            break;
        }
    }

    let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .or_else(|| {
            headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = header_end + 4;
    while buffer.len() < body_start + content_length {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    let mut lines = headers.lines();
    let request_line = lines.next().unwrap();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap().to_string();
    let path = parts.next().unwrap().to_string();
    let content_type = headers.lines().find_map(|line| {
        line.strip_prefix("content-type: ")
            .or_else(|| line.strip_prefix("Content-Type: "))
            .map(|value| value.trim().to_string())
    });
    let body =
        String::from_utf8_lossy(&buffer[body_start..body_start + content_length]).to_string();
    let bulk_action = body.lines().next().and_then(|line| {
        serde_json::from_str::<Value>(line)
            .ok()?
            .as_object()?
            .keys()
            .next()
            .cloned()
    });
    let is_update_bulk = bulk_action.as_deref() == Some("update");
    let is_create_bulk = bulk_action.as_deref() == Some("create");

    requests.lock().unwrap().push(RecordedRequest {
        method: method.clone(),
        path: path.clone(),
        content_type,
        body,
    });

    let (status, response_body) = if path.contains("/_bulk") {
        let response_body = if is_update_bulk {
            r#"{"errors":false,"items":[{"update":{"_index":"logs-docs","_id":"1","status":200,"result":"noop"}}]}"#
        } else if is_create_bulk {
            r#"{"errors":false,"items":[{"create":{"_index":"logs-docs","_id":"1","status":201}}]}"#
        } else {
            r#"{"errors":false,"items":[{"index":{"_index":"logs-docs","_id":"1","status":201}}]}"#
        };
        ("200 OK", response_body)
    } else if template_status == 200 {
        ("200 OK", r#"{"acknowledged":true}"#)
    } else {
        (
            "409 Conflict",
            r#"{"error":{"type":"resource_already_exists_exception","reason":"exists"},"status":409}"#,
        )
    };
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response_body}",
        response_body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

#[test]
fn cli_installs_template_before_bulk_with_default_name_and_put() {
    let dir = temp_dir("espipe-template-put");
    let input = write_input_file(&dir);
    let template = write_template_file(
        &dir,
        "logs-docs.json",
        r#"{"index_patterns":["logs-*"],"priority":1}"#,
    );
    let (base_url, requests) = spawn_server(200);

    let output = run_espipe(&[
        input.display().to_string(),
        format!("{base_url}/logs-docs"),
        "--template".to_string(),
        template.display().to_string(),
        "--uncompressed".to_string(),
        "--batch-size".to_string(),
        "1".to_string(),
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().unwrap();
    assert!(requests.len() >= 3, "requests: {requests:?}");
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(requests[0].path, "/_index_template/logs-docs");
    assert_eq!(
        requests[0].content_type.as_deref(),
        Some("application/json")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].body).unwrap()["priority"],
        1
    );
    assert!(
        requests[1..]
            .iter()
            .all(|request| request.path == "/logs-docs/_bulk")
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.path.contains("/_template/"))
    );
    let bulk_lines: Vec<Value> = requests[1]
        .body
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(bulk_lines[0].get("index").is_some());
}

#[test]
fn cli_installs_pipeline_then_template_then_bulk_when_template_references_pipeline() {
    let dir = temp_dir("espipe-template-pipeline");
    let input = write_input_file(&dir);
    let pipeline = write_pipeline_file(&dir, "geoip.json", r#"{"processors":[]}"#);
    let template = write_template_file(
        &dir,
        "logs-docs.json",
        r#"{"index_patterns":["logs-*"],"template":{"settings":{"index.default_pipeline":"geoip"}}}"#,
    );
    let (base_url, requests) = spawn_server(200);

    let output = run_espipe(&[
        input.display().to_string(),
        format!("{base_url}/logs-docs"),
        "--pipeline".to_string(),
        pipeline.display().to_string(),
        "--template".to_string(),
        template.display().to_string(),
        "--uncompressed".to_string(),
        "--batch-size".to_string(),
        "1".to_string(),
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().unwrap();
    assert!(requests.len() >= 4, "requests: {requests:?}");
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(requests[0].path, "/_ingest/pipeline/geoip");
    assert_eq!(requests[1].method, "PUT");
    assert_eq!(requests[1].path, "/_index_template/logs-docs");
    assert!(
        requests[2..]
            .iter()
            .all(|request| request.path == "/logs-docs/_bulk")
    );
}

#[test]
fn cli_installs_yaml_pipeline_and_template_as_json() {
    let dir = temp_dir("espipe-template-yaml");
    let input = write_input_file(&dir);
    let pipeline = write_pipeline_file(
        &dir,
        "geoip.yml",
        r#"
processors:
  - set:
      field: ingested_by
      value: yaml-pipeline
"#,
    );
    let template = write_template_file(
        &dir,
        "logs-docs.yml",
        r#"
index_patterns:
  - logs-*
template:
  settings:
    index.default_pipeline: geoip
"#,
    );
    let (base_url, requests) = spawn_server(200);

    let output = run_espipe(&[
        input.display().to_string(),
        format!("{base_url}/logs-docs"),
        "--pipeline".to_string(),
        pipeline.display().to_string(),
        "--template".to_string(),
        template.display().to_string(),
        "--uncompressed".to_string(),
        "--batch-size".to_string(),
        "1".to_string(),
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().unwrap();
    assert!(requests.len() >= 4, "requests: {requests:?}");
    assert_eq!(requests[0].path, "/_ingest/pipeline/geoip");
    assert_eq!(
        requests[0].content_type.as_deref(),
        Some("application/json")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].body).unwrap()["processors"][0]["set"]["value"],
        "yaml-pipeline"
    );
    assert_eq!(requests[1].path, "/_index_template/logs-docs");
    assert_eq!(
        requests[1].content_type.as_deref(),
        Some("application/json")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[1].body).unwrap()["template"]["settings"]["index.default_pipeline"],
        "geoip"
    );
}

#[test]
fn template_default_pipeline_is_checked_when_pipeline_file_is_omitted() {
    let dir = temp_dir("espipe-template-pipeline-exists");
    let input = write_input_file(&dir);
    let template = write_template_file(
        &dir,
        "logs-docs.json",
        r#"{"index_patterns":["logs-*"],"template":{"settings":{"index.default_pipeline":"existing-pipeline"}}}"#,
    );
    let (base_url, requests) = spawn_server(200);

    let output = run_espipe(&[
        input.display().to_string(),
        format!("{base_url}/logs-docs"),
        "--template".to_string(),
        template.display().to_string(),
        "--uncompressed".to_string(),
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().unwrap();
    assert!(requests.len() >= 3, "requests: {requests:?}");
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/_ingest/pipeline/existing-pipeline");
    assert_eq!(requests[1].method, "PUT");
    assert_eq!(requests[1].path, "/_index_template/logs-docs");
    assert!(
        requests[2..]
            .iter()
            .all(|request| request.path == "/logs-docs/_bulk")
    );
}

#[test]
fn cli_globs_fixture_documents_with_pipeline_and_template() {
    let dir = temp_dir("espipe-glob-template-pipeline");
    let input_pattern = fixture_path("glob_docs").join("**").join("*.md");
    let pipeline = write_pipeline_file(&dir, "glob-pipeline.json", r#"{"processors":[]}"#);
    let template = write_template_file(
        &dir,
        "glob-template.json",
        r#"{"index_patterns":["glob-docs"],"template":{"settings":{"index.default_pipeline":"glob-pipeline"}}}"#,
    );
    let (base_url, requests) = spawn_server(200);

    let output = run_espipe(&[
        input_pattern.display().to_string(),
        format!("{base_url}/glob-docs"),
        "--pipeline".to_string(),
        pipeline.display().to_string(),
        "--template".to_string(),
        template.display().to_string(),
        "--content".to_string(),
        "markdown".to_string(),
        "--uncompressed".to_string(),
        "--batch-size".to_string(),
        "10".to_string(),
        "--quiet".to_string(),
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().unwrap();
    assert!(requests.len() >= 3, "requests: {requests:?}");
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(requests[0].path, "/_ingest/pipeline/glob-pipeline");
    assert_eq!(requests[1].method, "PUT");
    assert_eq!(requests[1].path, "/_index_template/glob-template");
    assert!(
        requests[2..]
            .iter()
            .all(|request| request.path == "/glob-docs/_bulk")
    );

    let bulk_body = requests[2..]
        .iter()
        .map(|request| request.body.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(bulk_body.contains(r#""filename":"alpha.md""#));
    assert!(bulk_body.contains(r#""filename":"bravo.md""#));
    assert!(bulk_body.contains(r#""filename":"charlie.md""#));
    assert!(bulk_body.contains(r#""filename":"delta.md""#));
    assert!(!bulk_body.contains("ignored.tmp"));
    assert!(bulk_body.contains("\"markdown\":\"# Alpha\\n\\nFirst document."));
    assert!(bulk_body.contains("\"markdown\":\"# Bravo\\n\\nSecond document."));
    assert!(bulk_body.contains("\"markdown\":\"# Charlie\\n\\nThird document."));
    assert!(bulk_body.contains("\"markdown\":\"# Delta\\n\\nNested document."));
    assert!(bulk_body.contains(r#""order":1"#));
    assert!(bulk_body.contains(r#""order":2"#));
    assert!(bulk_body.contains(r#""order":3"#));
    assert!(bulk_body.contains(r#""order":4"#));
}

#[test]
fn cli_upsert_reuses_file_ids_after_content_changes_and_file_additions() {
    let dir = temp_workspace_dir("espipe-upsert-files");
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    fs::write(&first, "first version").unwrap();
    let (base_url, requests) = spawn_server(200);

    let run = |input: &str| {
        run_espipe(&[
            input.to_string(),
            format!("{base_url}/logs-docs"),
            "--action".to_string(),
            "upsert".to_string(),
            "--generate-id=true".to_string(),
            "--uncompressed".to_string(),
            "--quiet".to_string(),
        ])
    };

    let first_output = run(&first.display().to_string());
    assert!(
        first_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first_output.stderr)
    );
    let first_body = requests
        .lock()
        .unwrap()
        .iter()
        .find(|request| request.path == "/logs-docs/_bulk")
        .map(|request| request.body.clone())
        .expect("first bulk request");
    let first_lines: Vec<Value> = first_body
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(first_lines.len(), 2);
    assert!(first_lines[0].get("update").is_some());
    assert_eq!(first_lines[1]["doc_as_upsert"], true);
    assert!(first_lines[1]["doc"].get("_id").is_none());
    assert_eq!(first_lines[1]["doc"]["origin"]["scheme"], "file");
    let first_id = first_lines[0]["update"]["_id"]
        .as_str()
        .unwrap()
        .to_string();

    fs::write(&first, "changed version").unwrap();
    fs::write(&second, "new file").unwrap();
    let pattern = dir.path().join("*.md").display().to_string();
    let second_output = run(&pattern);
    assert!(
        second_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&second_output.stderr)
    );

    let bulk_bodies: Vec<String> = requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.path == "/logs-docs/_bulk")
        .map(|request| request.body.clone())
        .collect();
    assert_eq!(bulk_bodies.len(), 2);
    let second_lines: Vec<Value> = bulk_bodies[1]
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(second_lines.len(), 4);
    assert_eq!(second_lines[0]["update"]["_id"], first_id);
    assert_ne!(
        second_lines[0]["update"]["_id"],
        second_lines[2]["update"]["_id"]
    );
}

#[test]
fn cli_generate_id_false_omits_ids_for_index() {
    let dir = temp_dir("espipe-no-generated-id");
    let input = dir.join("document.md");
    fs::write(&input, "document").unwrap();
    let (base_url, requests) = spawn_server(200);

    for action in ["create", "index"] {
        let output = run_espipe(&[
            input.display().to_string(),
            format!("{base_url}/logs-docs"),
            "--action".to_string(),
            action.to_string(),
            "--generate-id=false".to_string(),
            "--uncompressed".to_string(),
            "--quiet".to_string(),
        ]);

        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let body = requests
            .lock()
            .unwrap()
            .iter()
            .rfind(|request| request.path == "/logs-docs/_bulk")
            .map(|request| request.body.clone())
            .expect("bulk request")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let expected_metadata = match action {
            "create" => serde_json::json!({"create": {}}),
            "index" => serde_json::json!({"index": {}}),
            _ => unreachable!(),
        };
        assert_eq!(body[0], expected_metadata);
        assert!(body[1].get("_id").is_none());
    }
}

#[test]
fn cli_generate_id_false_rejects_update_and_upsert_without_explicit_id() {
    let dir = temp_dir("espipe-no-generated-upsert-id");
    let input = dir.join("document.md");
    fs::write(&input, "document").unwrap();

    for action in ["update", "upsert"] {
        let (base_url, requests) = spawn_server(200);
        let output = run_espipe(&[
            input.display().to_string(),
            format!("{base_url}/logs-docs"),
            "--action".to_string(),
            action.to_string(),
            "--generate-id=false".to_string(),
            "--uncompressed".to_string(),
            "--quiet".to_string(),
        ]);

        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("action requires"));
        assert!(
            !requests
                .lock()
                .unwrap()
                .iter()
                .any(|request| request.path == "/logs-docs/_bulk")
        );
    }
}

#[test]
fn cli_uses_create_only_post_when_overwrite_is_false() {
    let dir = temp_dir("espipe-template-post");
    let input = write_input_file(&dir);
    let template = write_template_file(&dir, "logs.json", r#"{"index_patterns":["logs-*"]}"#);
    let (base_url, requests) = spawn_server(200);

    let output = run_espipe(&[
        input.display().to_string(),
        format!("{base_url}/logs-2026"),
        "--template".to_string(),
        template.display().to_string(),
        "--template-name".to_string(),
        "custom-template".to_string(),
        "--template-overwrite=false".to_string(),
        "--uncompressed".to_string(),
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        requests[0].path,
        "/_index_template/custom-template?create=true"
    );
}

#[test]
fn cli_without_template_sends_only_bulk_requests() {
    let dir = temp_dir("espipe-template-omitted");
    let input = write_input_file(&dir);
    let (base_url, requests) = spawn_server(200);

    let output = run_espipe(&[
        input.display().to_string(),
        format!("{base_url}/logs-docs"),
        "--uncompressed".to_string(),
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().unwrap();
    assert!(!requests.is_empty());
    assert!(
        requests
            .iter()
            .all(|request| request.path == "/logs-docs/_bulk")
    );
}

#[test]
fn no_template_file_output_preserves_input_first_failure_order() {
    let dir = temp_dir("espipe-template-no-template-order");
    let missing_input = dir.join("missing.ndjson");
    let output_path = dir.join("out.ndjson");

    let output = run_espipe(&[
        missing_input.display().to_string(),
        output_path.display().to_string(),
    ]);

    assert!(!output.status.success());
    assert!(
        !output_path.exists(),
        "file output should not be created when input open fails without a template"
    );
}

#[test]
fn rejected_template_aborts_before_bulk() {
    let dir = temp_dir("espipe-template-reject");
    let input = write_input_file(&dir);
    let template = write_template_file(&dir, "logs.json", r#"{"index_patterns":["logs-*"]}"#);
    let (base_url, requests) = spawn_server(409);

    let output = run_espipe(&[
        input.display().to_string(),
        format!("{base_url}/logs-2026"),
        "--template".to_string(),
        template.display().to_string(),
        "--template-overwrite=false".to_string(),
        "--uncompressed".to_string(),
    ]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("status 409"), "stderr: {stderr}");
    assert!(
        stderr.contains("resource_already_exists_exception"),
        "stderr: {stderr}"
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "requests: {requests:?}");
    assert!(
        !requests
            .iter()
            .any(|request| request.path.contains("_bulk"))
    );
}

#[test]
fn invalid_template_arguments_fail_before_input_access() {
    let dir = temp_dir("espipe-template-invalid-args");
    let template = write_template_file(&dir, "logs.json", r#"{"index_patterns":["logs-*"]}"#);
    let missing_input = dir.join("missing.ndjson");
    let output_path = dir.join("out.ndjson");

    let output = run_espipe(&[
        missing_input.display().to_string(),
        output_path.display().to_string(),
        "--template".to_string(),
        template.display().to_string(),
    ]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("template options require an Elasticsearch output"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("missing.ndjson"), "stderr: {stderr}");

    let output = run_espipe(&[
        missing_input.display().to_string(),
        "-".to_string(),
        "--template".to_string(),
        template.display().to_string(),
    ]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("template options require an Elasticsearch output")
    );
}

#[test]
fn none_pipeline_target_is_rejected_with_template() {
    let dir = temp_dir("espipe-template-none-pipeline");
    let template = write_template_file(&dir, "logs.json", r#"{"index_patterns":["logs-*"]}"#);
    let missing_input = dir.join("missing.ndjson");
    let output_path = dir.join("out.ndjson");

    let output = run_espipe(&[
        missing_input.display().to_string(),
        output_path.display().to_string(),
        "--template".to_string(),
        template.display().to_string(),
        "--pipeline-name".to_string(),
        "_none".to_string(),
    ]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("_none"), "stderr: {stderr}");
    assert!(stderr.contains("--template"), "stderr: {stderr}");
    assert!(!stderr.contains("missing.ndjson"), "stderr: {stderr}");
}

#[test]
fn template_name_and_overwrite_require_template() {
    let dir = temp_dir("espipe-template-requires");
    let input = write_input_file(&dir);
    let output_path = dir.join("out.ndjson");

    let output = run_espipe(&[
        input.display().to_string(),
        output_path.display().to_string(),
        "--template-name".to_string(),
        "custom".to_string(),
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--template"));

    let output = run_espipe(&[
        input.display().to_string(),
        output_path.display().to_string(),
        "--template-overwrite=false".to_string(),
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--template"));
}

#[test]
fn template_parse_failures_are_path_specific() {
    let dir = temp_dir("espipe-template-parse");
    let input = write_input_file(&dir);
    let invalid_json = write_template_file(
        &dir,
        "bad.json",
        r#"{"index_patterns":["logs-*"] /* invalid */}"#,
    );
    let invalid_json5 = write_template_file(&dir, "bad.json5", r#"{index_patterns:["logs-*"],"#);
    let (base_url, _requests) = spawn_server(200);

    for template in [invalid_json, invalid_json5] {
        let output = run_espipe(&[
            input.display().to_string(),
            format!("{base_url}/logs-2026"),
            "--template".to_string(),
            template.display().to_string(),
        ]);
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&template.display().to_string()),
            "stderr: {stderr}"
        );
        assert!(
            stderr.contains("failed to parse template"),
            "stderr: {stderr}"
        );
    }
}

#[test]
fn jsonc_and_json5_templates_are_sent_as_json() {
    let dir = temp_dir("espipe-template-json5");
    let input = write_input_file(&dir);
    let jsonc = write_template_file(
        &dir,
        "logs.jsonc",
        r#"{"index_patterns":["logs-*"], /* comment */ "priority": 7}"#,
    );
    let json5 = write_template_file(
        &dir,
        "logs5.json5",
        r#"{index_patterns:["logs-*"], template: { settings: { number_of_shards: 1 } }}"#,
    );

    for template in [jsonc, json5] {
        let (base_url, requests) = spawn_server(200);
        let output = run_espipe(&[
            input.display().to_string(),
            format!("{base_url}/logs-2026"),
            "--template".to_string(),
            template.display().to_string(),
            "--uncompressed".to_string(),
        ]);
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let request = requests.lock().unwrap()[0].clone();
        serde_json::from_str::<Value>(&request.body).expect("normalized json body");
        assert!(!request.body.contains("/*"));
    }
}

#[test]
fn index_pattern_mismatch_warns_without_aborting() {
    let dir = temp_dir("espipe-template-pattern-mismatch");
    let input = write_input_file(&dir);
    let template = write_template_file(&dir, "metrics.json", r#"{"index_patterns":["metrics-*"]}"#);
    let (base_url, requests) = spawn_server(200);

    let output = run_espipe(&[
        input.display().to_string(),
        format!("{base_url}/logs-2026"),
        "--template".to_string(),
        template.display().to_string(),
        "--uncompressed".to_string(),
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("template index_patterns do not match target index 'logs-2026'"),
        "stderr: {stderr}"
    );
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.path == "/logs-2026/_bulk")
    );
}

#[test]
fn unverifiable_index_patterns_warn_without_aborting() {
    let dir = temp_dir("espipe-template-pattern-unverifiable");
    let input = write_input_file(&dir);
    let template = write_template_file(&dir, "logs.json", r#"{"template":{"settings":{}}}"#);
    let (base_url, requests) = spawn_server(200);

    let output = run_espipe(&[
        input.display().to_string(),
        format!("{base_url}/logs-2026"),
        "--template".to_string(),
        template.display().to_string(),
        "--uncompressed".to_string(),
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("could not verify template index_patterns for target index 'logs-2026'"),
        "stderr: {stderr}"
    );
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.path == "/logs-2026/_bulk")
    );
}
