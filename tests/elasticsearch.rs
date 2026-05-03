use elasticsearch::http::{
    Method,
    headers::{HeaderMap, HeaderValue},
};
use elasticsearch::indices::{IndicesDeleteParts, IndicesRefreshParts};
use elasticsearch::{
    CountParts, Elasticsearch,
    http::transport::{SingleNodeConnectionPool, TransportBuilder},
};
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

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_ingests_into_elasticsearch_if_available() -> Result<()> {
    let base_url = Url::parse("http://localhost:9200")?;
    let transport =
        TransportBuilder::new(SingleNodeConnectionPool::new(base_url.clone())).build()?;
    let client = Elasticsearch::new(transport);

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a local Elasticsearch node at http://localhost:9200"]
async fn cli_ingests_gzip_ndjson_fixture_into_localhost() -> Result<()> {
    let base_url = Url::parse("http://localhost:9200")?;
    let transport =
        TransportBuilder::new(SingleNodeConnectionPool::new(base_url.clone())).build()?;
    let client = Elasticsearch::new(transport);

    if !is_connected(&client).await.unwrap_or(false) {
        eprintln!("Skipping Elasticsearch integration test; local node not available.");
        return Ok(());
    }

    let input_path = fixture_path("compressed.ndjson.gz");
    let index = test_index_name();
    let output_url = format!("{}/{}", base_url.as_str().trim_end_matches('/'), index);

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&input_path)
        .arg(&output_url)
        .output()
        .expect("run espipe");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    client
        .indices()
        .refresh(IndicesRefreshParts::Index(&[&index]))
        .send()
        .await?;

    let response = client.count(CountParts::Index(&[&index])).send().await?;
    let body: Value = response.json().await?;
    let count = body.get("count").and_then(Value::as_u64).unwrap_or(0);
    assert_eq!(count, 1000);

    client
        .indices()
        .delete(IndicesDeleteParts::Index(&[&index]))
        .send()
        .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a local Elasticsearch node at http://localhost:9200"]
async fn cli_ingests_fixture_with_pipeline_and_template_into_localhost() -> Result<()> {
    let base_url = Url::parse("http://localhost:9200")?;
    let transport =
        TransportBuilder::new(SingleNodeConnectionPool::new(base_url.clone())).build()?;
    let client = Elasticsearch::new(transport);

    if !is_connected(&client).await.unwrap_or(false) {
        eprintln!("Skipping Elasticsearch integration test; local node not available.");
        return Ok(());
    }

    let temp_dir = temp_dir("espipe-es-template-pipeline-it");
    let input_path = fixture_path("pipeline_template_input.ndjson");
    let index = test_index_name();
    let pipeline_name = format!("{index}-pipeline");
    let template_name = format!("{index}-template");
    let output_url = format!("{}/{}", base_url.as_str().trim_end_matches('/'), index);
    let pipeline_path = temp_dir.join("fixture-pipeline.json");
    let template_path = temp_dir.join("fixture-template.json");

    fs::write(
        &pipeline_path,
        r#"{"description":"espipe localhost fixture pipeline","processors":[{"set":{"field":"ingested_by","value":"espipe-localhost-pipeline"}},{"set":{"field":"fixture_pipeline_applied","value":true}}]}"#,
    )?;
    fs::write(
        &template_path,
        format!(
            r#"{{
  "index_patterns": ["{index}"],
  "template": {{
    "settings": {{
      "index.default_pipeline": "{pipeline_name}",
      "number_of_shards": 1,
      "number_of_replicas": 0
    }},
    "mappings": {{
      "properties": {{
        "id": {{"type": "integer"}},
        "value": {{"type": "integer"}},
        "category": {{"type": "keyword"}},
        "ingested_by": {{"type": "keyword"}},
        "fixture_pipeline_applied": {{"type": "boolean"}}
      }}
    }}
  }}
}}"#
        ),
    )?;

    let output = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&input_path)
        .arg(&output_url)
        .arg("--pipeline")
        .arg(&pipeline_path)
        .arg("--pipeline-name")
        .arg(&pipeline_name)
        .arg("--template")
        .arg(&template_path)
        .arg("--template-name")
        .arg(&template_name)
        .output()
        .expect("run espipe");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    client
        .indices()
        .refresh(IndicesRefreshParts::Index(&[&index]))
        .send()
        .await?;

    let response = client.count(CountParts::Index(&[&index])).send().await?;
    let body: Value = response.json().await?;
    let count = body.get("count").and_then(Value::as_u64).unwrap_or(0);
    assert_eq!(count, 100);

    let transformed_count = count_pipeline_field(&client, &index).await?;
    assert_eq!(transformed_count, 100);

    cleanup_elasticsearch_resource(&client, Method::Delete, &format!("/{index}")).await?;
    cleanup_elasticsearch_resource(
        &client,
        Method::Delete,
        &format!("/_index_template/{template_name}"),
    )
    .await?;
    cleanup_elasticsearch_resource(
        &client,
        Method::Delete,
        &format!("/_ingest/pipeline/{pipeline_name}"),
    )
    .await?;

    Ok(())
}

async fn count_pipeline_field(client: &Elasticsearch, index: &str) -> Result<u64> {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    let body = br#"{"query":{"term":{"ingested_by":"espipe-localhost-pipeline"}}}"#.to_vec();
    let response = client
        .send(
            Method::Post,
            &format!("/{index}/_count"),
            headers,
            Option::<&()>::None,
            Some(body),
            None,
        )
        .await?;
    let body: Value = response.json().await?;
    Ok(body.get("count").and_then(Value::as_u64).unwrap_or(0))
}

async fn cleanup_elasticsearch_resource(
    client: &Elasticsearch,
    method: Method,
    path: &str,
) -> Result<()> {
    let response = client
        .send(
            method,
            path,
            HeaderMap::new(),
            Option::<&()>::None,
            Option::<Vec<u8>>::None,
            None,
        )
        .await?;
    if response.status_code().as_u16() == 404 {
        return Ok(());
    }
    response.error_for_status_code()?;
    Ok(())
}

async fn is_connected(client: &Elasticsearch) -> Result<bool> {
    let response = match client.info().send().await {
        Ok(response) => response,
        Err(_) => return Ok(false),
    };

    let body: Value = match response.json().await {
        Ok(body) => body,
        Err(_) => return Ok(false),
    };

    Ok(body
        .get("tagline")
        .and_then(Value::as_str)
        .is_some_and(|tagline| tagline == "You Know, for Search"))
}
