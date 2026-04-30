use std::process::Command;

#[test]
fn cli_prints_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg("--version")
        .output()
        .expect("run espipe");

    assert!(output.status.success(), "espipe --version should succeed");
    assert!(
        output.stderr.is_empty(),
        "espipe --version should not write stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("espipe {}\n", env!("CARGO_PKG_VERSION"))
    );
}
