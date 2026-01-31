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
