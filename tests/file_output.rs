use base64::Engine as _;
use flate2::read::GzDecoder;
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn json_lines(output: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(output)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("JSON output line"))
        .collect()
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn temp_output_path(filename: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("espipe-test-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir.join(filename)
}

fn temp_workspace_path(filename: &str) -> (tempfile::TempDir, PathBuf) {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    fs::create_dir_all(&target_dir).expect("create target directory");
    let dir = tempfile::Builder::new()
        .prefix("espipe-test-")
        .tempdir_in(target_dir)
        .expect("create workspace temp dir");
    let path = dir.path().join(filename);
    (dir, path)
}

fn write_base64_fixture(name: &str, path: &Path) {
    let encoded = fs::read_to_string(fixture_path(name)).expect("read base64 fixture");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .expect("decode base64 fixture");
    fs::write(path, bytes).expect("write decoded fixture");
}

#[test]
fn cli_splits_root_map_to_ndjson_file() {
    let input_path = fixture_path("split_root_map.json");
    let output_path = temp_output_path("split-root-map.ndjson");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&input_path)
        .arg(&output_path)
        .arg("--split")
        .arg("/")
        .output()
        .expect("run espipe");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let documents = json_lines(&fs::read(&output_path).expect("read split output"));
    assert_eq!(documents.len(), 2);
    assert!(documents.iter().any(|document| {
        document["id"] == "20"
            && document["name"] == "Beta"
            && document["origin"]["scheme"] == "file"
    }));
    assert!(documents.iter().any(|document| {
        document["id"] == "10"
            && document["name"] == "Alpha"
            && document["nested"] == serde_json::json!({"enabled": true})
            && document["origin"]["scheme"] == "file"
    }));
}

#[test]
fn cli_splits_wrapped_array_to_stdout() {
    let input_path = fixture_path("split_nested_array.json");
    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&input_path)
        .arg("-")
        .arg("--split")
        .arg("/hits/")
        .arg("--quiet")
        .output()
        .expect("run espipe");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let documents = json_lines(&output.stdout);
    assert_eq!(documents.len(), 2);
    assert!(documents.iter().any(|document| {
        document["id"] == "alpha"
            && document["name"] == "Alpha"
            && document["origin"]["scheme"] == "file"
    }));
    assert!(documents.iter().any(|document| {
        document["id"] == "beta"
            && document["name"] == "Beta"
            && document["tags"] == serde_json::json!(["featured"])
            && document["origin"]["scheme"] == "file"
    }));
}

#[test]
fn cli_splits_file_uri_and_stdin_sources() {
    let input_path = fixture_path("split_nested_array.json");
    let file_uri = format!("file://{}", input_path.display());
    let file_output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(file_uri)
        .arg("-")
        .arg("--split")
        .arg("/hits")
        .arg("--quiet")
        .output()
        .expect("run espipe with file URI");
    assert!(file_output.status.success());
    assert_eq!(json_lines(&file_output.stdout).len(), 2);

    let mut child = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg("-")
        .arg("-")
        .arg("--split")
        .arg("/")
        .arg("--quiet")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run espipe with split stdin");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"[{"name":"stdin"}]"#)
        .unwrap();
    let stdin_output = child.wait_with_output().unwrap();
    assert!(stdin_output.status.success());
    assert_eq!(
        json_lines(&stdin_output.stdout),
        vec![serde_json::json!({"name": "stdin"})]
    );
}

#[test]
fn cli_closes_gzip_output_after_late_split_error() {
    const COMPLETE_DOCUMENTS: usize = 128;

    let output_path = temp_output_path("late-split-error.ndjson.gz");
    let mut child = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg("-")
        .arg(&output_path)
        .arg("--split")
        .arg("/")
        .arg("--quiet")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run espipe with split stdin");

    let mut stdin = child.stdin.take().expect("open espipe stdin");
    let mut prefix = String::from("[");
    for index in 0..COMPLETE_DOCUMENTS {
        if index > 0 {
            prefix.push(',');
        }
        let mut state = index as u64 + 1;
        let padding = (0..1024)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                const ALPHANUMERIC: &[u8] =
                    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
                char::from(ALPHANUMERIC[(state as usize) % ALPHANUMERIC.len()])
            })
            .collect::<String>();
        prefix.push_str(&format!(r#"{{"value":{index},"padding":"{padding}"}}"#));
    }
    prefix.push(',');
    stdin
        .write_all(prefix.as_bytes())
        .expect("write complete split batch");
    stdin.flush().expect("flush complete split batch");

    let deadline = Instant::now() + Duration::from_secs(5);
    while !fs::metadata(&output_path).is_ok_and(|metadata| metadata.len() > 0)
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        fs::metadata(&output_path).is_ok_and(|metadata| metadata.len() > 0),
        "espipe did not write the complete split batch before the deadline"
    );

    stdin
        .write_all(br#"{"bad":}]"#)
        .expect("write malformed split suffix");
    drop(stdin);

    let output = child.wait_with_output().expect("wait for espipe");
    assert!(!output.status.success(), "malformed JSON should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("array element 128"),
        "stderr should retain the split error: {stderr}"
    );

    let compressed = fs::read(&output_path).expect("read gzip split output");
    let mut decoder = GzDecoder::new(compressed.as_slice());
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .expect("late-error output should be a complete gzip stream");
    assert_eq!(json_lines(&decoded).len(), COMPLETE_DOCUMENTS);
}

#[test]
fn cli_splits_each_multi_source_file_before_writing() {
    let (_workspace, second_input) = temp_workspace_path("split_root_map_copy.json");
    fs::copy(fixture_path("split_root_map.json"), &second_input).expect("copy second split input");
    let output_path = temp_output_path("split-multiple.ndjson");
    fs::write(&output_path, "preserve me").expect("write output sentinel");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(fixture_path("split_root_map.json"))
        .arg(&second_input)
        .arg(&output_path)
        .arg("--split")
        .arg("/")
        .output()
        .expect("run espipe");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let documents = json_lines(&fs::read(&output_path).expect("read split output"));
    assert_eq!(documents.len(), 4);
    assert!(documents.iter().all(|document| {
        document["origin"]["scheme"] == "file" && document["origin"]["filename"].is_string()
    }));
}

#[test]
fn cli_rejects_invalid_split_path_before_writing() {
    let output_path = temp_output_path("split-invalid-path.ndjson");
    fs::write(&output_path, "preserve me").expect("write output sentinel");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(fixture_path("split_root_map.json"))
        .arg(&output_path)
        .arg("--split")
        .arg("hits")
        .output()
        .expect("run espipe");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must start with '/'"));
    assert_eq!(fs::read_to_string(output_path).unwrap(), "preserve me");
}

#[test]
fn cli_preserves_json_behavior_without_split() {
    let input_path = fixture_path("split_root_map.json");
    let output_path = temp_output_path("unsplit-json.ndjson");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&input_path)
        .arg(&output_path)
        .output()
        .expect("run espipe");

    assert!(output.status.success());
    let document: Value = serde_json::from_slice(&fs::read(output_path).unwrap()).unwrap();
    assert!(document.get("20").is_some());
    assert!(document.get("10").is_some());
}

#[test]
fn cli_preserves_ndjson_stdin_without_split() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg("-")
        .arg("-")
        .arg("--quiet")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run espipe");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"message\":\"hello\"}\n{\"message\":\"world\"}\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert_eq!(json_lines(&output.stdout).len(), 2);
}

#[test]
fn cli_writes_local_ndjson_with_origin_to_file() {
    let input_path = fixture_path("bulk_input.ndjson");
    let output_path = temp_output_path("bulk_output.ndjson");

    let status = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(input_path)
        .arg(&output_path)
        .status()
        .expect("run espipe");

    assert!(status.success(), "espipe exited with failure");

    let contents = fs::read_to_string(&output_path).expect("read output file");
    let documents = json_lines(contents.as_bytes());
    assert_eq!(documents.len(), 4);
    for document in documents {
        assert_eq!(document["origin"]["scheme"], "file");
        assert_eq!(document["origin"]["filename"], "bulk_input.ndjson");
    }
}

#[test]
fn cli_converts_anydoc_pdf_to_existing_file_document_output() {
    let input_path = temp_output_path("sample.pdf");
    write_base64_fixture("anydoc/sample.pdf.base64", &input_path);
    let output_path = temp_output_path("anydoc.ndjson");

    let status = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(input_path)
        .arg(&output_path)
        .status()
        .expect("run espipe");

    assert!(status.success(), "espipe exited with failure");

    let contents = fs::read_to_string(&output_path).expect("read output file");
    let document: Value = serde_json::from_str(contents.trim()).expect("document json");
    assert!(
        document["content"]["body"]
            .as_str()
            .expect("body string")
            .contains("Hello PDF")
    );
    assert_eq!(document["origin"]["scheme"], "file");
    assert_eq!(document["origin"]["filename"], "sample.pdf");
    assert!(document.get("file").is_none());
}

#[test]
fn cli_converts_mixed_anydoc_and_markdown_inputs_without_changing_shape() {
    let (_workspace, anydoc_path) = temp_workspace_path("sample.pdf");
    write_base64_fixture("anydoc/sample.pdf.base64", &anydoc_path);
    let markdown_path = fixture_path("glob_docs").join("alpha.md");
    let output_path = temp_output_path("mixed-anydoc.ndjson");

    let status = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(anydoc_path)
        .arg(markdown_path)
        .arg(&output_path)
        .arg("--content")
        .arg("markdown")
        .status()
        .expect("run espipe");

    assert!(status.success(), "espipe exited with failure");

    let contents = fs::read_to_string(&output_path).expect("read output file");
    let documents: Vec<Value> = contents
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("document json"))
        .collect();

    assert_eq!(documents.len(), 2);
    assert!(documents.iter().any(|document| {
        document["content"]["markdown"]
            .as_str()
            .is_some_and(|body| body.contains("Hello PDF"))
    }));
    assert!(documents.iter().any(|document| {
        document["content"]["markdown"]
            .as_str()
            .is_some_and(|body| body.contains("Alpha"))
    }));
    assert!(documents.iter().all(|document| {
        document.get("file").is_none()
            && document["origin"]["scheme"] == "file"
            && document["origin"]["path"].is_string()
            && document["origin"]["filename"].is_string()
    }));
}

#[test]
fn cli_reports_anydoc_conversion_errors_on_stderr_with_source_path() {
    let input_path = temp_output_path("invalid.pdf");
    let output_path = temp_output_path("invalid-anydoc.ndjson");
    fs::write(&input_path, b"not a PDF").expect("write invalid input");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&input_path)
        .arg(&output_path)
        .output()
        .expect("run espipe");

    assert!(
        !output.status.success(),
        "espipe should reject invalid input"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid.pdf"),
        "stderr should identify input"
    );
    let (_, detail) = stderr
        .split_once("invalid.pdf")
        .expect("stderr should include error detail after the source path");
    assert!(
        !detail.trim().is_empty(),
        "stderr should include error detail beyond the source path"
    );
}

#[test]
fn cli_reports_image_only_pdf_requires_ocr_on_stderr() {
    let input_path = temp_output_path("image-only.pdf");
    let output_path = temp_output_path("image-only-anydoc.ndjson");
    write_base64_fixture("anydoc/image-only.pdf.base64", &input_path);

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&input_path)
        .arg(&output_path)
        .output()
        .expect("run espipe");

    assert!(
        !output.status.success(),
        "espipe should reject image-only PDF"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("image-only.pdf"));
    assert!(stderr.contains("OCR is required"));
}

#[test]
fn cli_skips_image_only_pdf_and_continues_with_later_files() {
    let (_workspace, image_path) = temp_workspace_path("image-only.pdf");
    let sample_path = image_path.with_file_name("sample.pdf");
    write_base64_fixture("anydoc/image-only.pdf.base64", &image_path);
    write_base64_fixture("anydoc/sample.pdf.base64", &sample_path);
    let output_path = temp_output_path("skipped-image-only.ndjson");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&image_path)
        .arg(&sample_path)
        .arg(&output_path)
        .output()
        .expect("run espipe");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("image-only.pdf"));
    assert!(stderr.contains("skipping file"));

    let documents = json_lines(&fs::read(&output_path).expect("read output"));
    assert_eq!(documents.len(), 1);
    assert!(
        documents[0]["content"]["body"]
            .as_str()
            .is_some_and(|body| body.contains("Hello PDF"))
    );
}

#[test]
fn cli_skips_malformed_pdf_and_continues_with_later_files() {
    let (_workspace, invalid_path) = temp_workspace_path("invalid.pdf");
    let sample_path = invalid_path.with_file_name("sample.pdf");
    fs::write(&invalid_path, b"not a PDF").expect("write invalid input");
    write_base64_fixture("anydoc/sample.pdf.base64", &sample_path);
    let output_path = temp_output_path("skipped-invalid-pdf.ndjson");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&invalid_path)
        .arg(&sample_path)
        .arg(&output_path)
        .output()
        .expect("run espipe");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid.pdf"));
    assert!(stderr.contains("skipping file"));

    let documents = json_lines(&fs::read(&output_path).expect("read output"));
    assert_eq!(documents.len(), 1);
    assert!(
        documents[0]["content"]["body"]
            .as_str()
            .is_some_and(|body| body.contains("Hello PDF"))
    );
}

#[test]
fn cli_rejects_multi_file_input_to_non_ndjson_file_output_before_writing() {
    let first_input = fixture_path("glob_docs").join("alpha.md");
    let second_input = fixture_path("glob_docs").join("bravo.md");
    let output_path = temp_output_path("not-an-output.md");
    fs::write(&output_path, "preserve me").expect("write output sentinel");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(first_input)
        .arg(second_input)
        .arg(&output_path)
        .output()
        .expect("run espipe");

    assert!(!output.status.success(), "espipe should reject output");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(".ndjson"),
        "stderr should mention .ndjson: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(&output_path).expect("read output sentinel"),
        "preserve me"
    );
}

#[test]
fn cli_preserves_remote_input_error_for_multi_https_inputs() {
    let output_path = temp_output_path("not-an-output.md");
    fs::write(&output_path, "preserve me").expect("write output sentinel");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg("https://example.com/one.ndjson")
        .arg("https://example.com/two.ndjson")
        .arg(&output_path)
        .output()
        .expect("run espipe");

    assert!(!output.status.success(), "espipe should reject input");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Remote inputs cannot be combined with file imports"),
        "stderr should preserve remote-input error: {stderr}"
    );
    assert!(
        !stderr.contains(".ndjson"),
        "stderr should not report local file-output rule: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(&output_path).expect("read output sentinel"),
        "preserve me"
    );
}

#[test]
fn cli_warns_and_skips_when_later_file_document_read_fails() {
    let first_input = fixture_path("glob_docs").join("alpha.md");
    let (_workspace, bad_input) = temp_workspace_path("bad.txt");
    fs::write(&bad_input, [0xff]).expect("write invalid utf8 input");
    let output_path = temp_output_path("out.ndjson");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(first_input)
        .arg(&bad_input)
        .arg(&output_path)
        .output()
        .expect("run espipe");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not valid UTF-8"),
        "stderr should report read failure: {stderr}"
    );
    assert!(stderr.contains("skipping file"));
    let documents = json_lines(&fs::read(&output_path).expect("read output"));
    assert_eq!(documents.len(), 1);
    assert_eq!(
        documents[0]["content"]["body"],
        "# Alpha\n\nFirst document.\n"
    );
}

#[test]
fn cli_accepts_multi_file_input_to_ndjson_file_output() {
    let first_input = fixture_path("glob_docs").join("alpha.md");
    let second_input = fixture_path("glob_docs").join("bravo.md");
    let output_path = temp_output_path("glob_docs.ndjson");

    let status = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(first_input)
        .arg(second_input)
        .arg(&output_path)
        .status()
        .expect("run espipe");

    assert!(status.success(), "espipe exited with failure");
    let contents = fs::read_to_string(&output_path).expect("read output file");
    assert!(contents.contains(r#""filename":"alpha.md""#));
    assert!(contents.contains(r#""filename":"bravo.md""#));
    assert!(!contents.contains(r#""file":{"#));
}

#[test]
fn cli_accepts_multi_file_input_to_gzip_ndjson_file_output() {
    let first_input = fixture_path("glob_docs").join("alpha.md");
    let second_input = fixture_path("glob_docs").join("bravo.md");
    let output_path = temp_output_path("glob_docs.ndjson.gz");

    let status = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(first_input)
        .arg(second_input)
        .arg(&output_path)
        .status()
        .expect("run espipe");

    assert!(status.success(), "espipe exited with failure");

    let file = fs::File::open(&output_path).expect("open gzip output");
    let mut decoder = GzDecoder::new(file);
    let mut contents = String::new();
    decoder
        .read_to_string(&mut contents)
        .expect("decompress output");
    assert!(contents.contains(r#""filename":"alpha.md""#));
    assert!(contents.contains(r#""filename":"bravo.md""#));
    assert!(!contents.contains(r#""file":{"#));
}

#[test]
fn cli_rejects_unsupported_gzip_file_output_before_writing() {
    let input_path = fixture_path("bulk_input.ndjson");
    let output_path = temp_output_path("out.csv.gz");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(input_path)
        .arg(&output_path)
        .output()
        .expect("run espipe");

    assert!(!output.status.success(), "espipe should reject output");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Unsupported compressed output format"),
        "stderr should mention unsupported compressed output: {stderr}"
    );
    assert!(!output_path.exists());
}
