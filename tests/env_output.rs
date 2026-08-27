use std::{fs, process::Command};

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create temporary workspace");
    fs::write(dir.path().join("docs.ndjson"), "{\"message\":\"hello\"}\n")
        .expect("write input fixture");
    dir
}

fn run_espipe(dir: &tempfile::TempDir, environment_url: Option<&str>) -> std::process::Output {
    run_espipe_to(dir, "env:/logs", environment_url)
}

fn run_espipe_to(
    dir: &tempfile::TempDir,
    output: &str,
    environment_url: Option<&str>,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_espipe"));
    command
        .current_dir(dir.path())
        .env_remove("ELASTIC_ES_URL")
        .env_remove("ELASTIC_ES_API_KEY")
        .args(["docs.ndjson", output]);
    if let Some(url) = environment_url {
        command.env("ELASTIC_ES_URL", url);
    }
    command.output().expect("run espipe")
}

#[test]
fn env_output_fails_when_url_is_absent_from_environment_and_dotenv() {
    let dir = workspace();
    let output = run_espipe(&dir, None);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "env:/index outputs require ELASTIC_ES_URL\n"
    );
}

#[test]
fn env_output_reads_url_from_dotenv() {
    let dir = workspace();
    fs::write(
        dir.path().join(".env"),
        "ELASTIC_ES_URL=file:///dotenv-value\n",
    )
    .expect("write .env");

    let output = run_espipe(&dir, None);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("ELASTIC_ES_URL must be an absolute http:// or https:// URL")
    );
}

#[test]
fn process_environment_takes_precedence_over_dotenv() {
    let dir = workspace();
    fs::write(
        dir.path().join(".env"),
        "ELASTIC_ES_URL=https://127.0.0.1:1\n",
    )
    .expect("write .env");

    let output = run_espipe(&dir, Some("file:///process-environment-value"));

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("ELASTIC_ES_URL must be an absolute http:// or https:// URL")
    );
}

#[test]
fn malformed_dotenv_fails_environment_output() {
    let dir = workspace();
    fs::write(dir.path().join(".env"), "ELASTIC_ES_URL='unterminated\n")
        .expect("write malformed .env");

    let output = run_espipe(&dir, None);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Could not read .env"));
}

#[test]
fn invalid_environment_uri_is_rejected_before_dotenv_is_loaded() {
    let dir = workspace();
    fs::write(dir.path().join(".env"), "ELASTIC_ES_URL='unterminated\n")
        .expect("write malformed .env");

    for target in ["env:/", "env:logs", "env://logs"] {
        let output = run_espipe_to(&dir, target, None);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(!output.status.success());
        assert!(
            stderr.contains("environment outputs must use `env:/index`"),
            "stderr for {target}: {stderr}"
        );
        assert!(!stderr.contains("Could not read .env"));
    }
}

#[test]
fn non_environment_output_does_not_load_dotenv() {
    let dir = workspace();
    fs::write(dir.path().join(".env"), "ELASTIC_ES_URL='unterminated\n")
        .expect("write malformed .env");

    let output = run_espipe_to(&dir, "output.ndjson", None);

    assert!(output.status.success());
    assert!(
        fs::read_to_string(dir.path().join("output.ndjson"))
            .expect("read output")
            .contains("\"message\":\"hello\"")
    );
}
