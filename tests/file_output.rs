use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

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

fn validate_bulk_schema(lines: &[&str]) {
    assert!(
        lines.len() % 2 == 0,
        "bulk output should have even line count"
    );
    for pair in lines.chunks(2) {
        let action: Value = serde_json::from_str(pair[0]).expect("action json");
        let source: Value = serde_json::from_str(pair[1]).expect("source json");

        let action_obj = action.as_object().expect("action object");
        assert_eq!(
            action_obj.len(),
            1,
            "action line should contain a single action"
        );
        let (action_name, action_value) = action_obj.iter().next().expect("action entry");
        assert!(
            matches!(
                action_name.as_str(),
                "index" | "create" | "update" | "delete"
            ),
            "unexpected bulk action {action_name}"
        );
        assert!(
            action_value.is_object(),
            "bulk action metadata should be an object"
        );

        assert!(source.is_object(), "source line should be an object");
    }
}

#[test]
fn cli_writes_bulk_output_to_file() {
    let input_path = fixture_path("bulk_input.ndjson");
    let output_path = temp_output_path("bulk_output.ndjson");

    let status = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(input_path)
        .arg(&output_path)
        .status()
        .expect("run espipe");

    assert!(status.success(), "espipe exited with failure");

    let contents = fs::read_to_string(&output_path).expect("read output file");
    let lines: Vec<&str> = contents.lines().filter(|line| !line.is_empty()).collect();
    assert!(!lines.is_empty(), "output file should not be empty");

    validate_bulk_schema(&lines);
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
fn cli_exits_with_error_when_later_file_document_read_fails() {
    let first_input = fixture_path("glob_docs").join("alpha.md");
    let bad_input = temp_output_path("bad.txt");
    fs::write(&bad_input, [0xff]).expect("write invalid utf8 input");
    let output_path = temp_output_path("out.ndjson");

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(first_input)
        .arg(&bad_input)
        .arg(&output_path)
        .output()
        .expect("run espipe");

    assert!(!output.status.success(), "espipe should reject bad input");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not valid UTF-8"),
        "stderr should report read failure: {stderr}"
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
    assert!(contents.contains(r#""name":"alpha.md""#));
    assert!(contents.contains(r#""name":"bravo.md""#));
}
