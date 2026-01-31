use elasticsearch::indices::{IndicesDeleteParts, IndicesRefreshParts};
use elasticsearch::CountParts;
use espipe::client::ElasticsearchBuilder;
use espipe::client::elasticsearch::is_connected;
use eyre::Result;
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;

fn temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn test_index_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_nanos();
    format!("espipe-test-{}-{nanos}", std::process::id())
}

fn write_input_file(dir: &PathBuf, filename: &str) -> PathBuf {
    let path = dir.join(filename);
    let contents = r#"{"message":"hello"}
{"message":"world"}
"#;
    fs::write(&path, contents).expect("write input file");
    path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_ingests_into_elasticsearch_if_available() -> Result<()> {
    let base_url = Url::parse("http://localhost:9200")?;
    let client = ElasticsearchBuilder::new(base_url.clone()).build()?;

    if !is_connected(&client).await.unwrap_or(false) {
        eprintln!("Skipping Elasticsearch integration test; local node not available.");
        return Ok(());
    }

    let temp_dir = temp_dir("espipe-es-it");
    let input_path = write_input_file(&temp_dir, "bulk_input.ndjson");
    let index = test_index_name();
    let output_url = format!("{}/{}", base_url.as_str().trim_end_matches('/'), index);

    let status = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&input_path)
        .arg(&output_url)
        .status()
        .expect("run espipe");

    assert!(status.success(), "espipe exited with failure");

    client
        .indices()
        .refresh(IndicesRefreshParts::Index(&[&index]))
        .send()
        .await?;

    let response = client.count(CountParts::Index(&[&index])).send().await?;
    let body: Value = response.json().await?;
    let count = body.get("count").and_then(Value::as_u64).unwrap_or(0);
    assert_eq!(count, 2);

    client
        .indices()
        .delete(IndicesDeleteParts::Index(&[&index]))
        .send()
        .await?;

    Ok(())
}
