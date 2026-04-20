use serde_json::Value;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const BENCH_DOCS: usize = 525_000;
const BENCH_BYTES_MIN: u64 = 100 * 1024 * 1024;
const NGINX_BENCH_DOCS: usize = 10_000_000;
const NGINX_BENCH_BYTES_MIN: u64 = 2 * 1024 * 1024 * 1024;

#[test]
#[ignore = "requires localhost:9200 and is intended for manual benchmarking"]
fn localhost_large_ingest_benchmark() {
    let fixture = benchmark_fixture().unwrap();
    let fixture_size = fs::metadata(&fixture).unwrap().len();
    assert!(fixture_size >= BENCH_BYTES_MIN);

    let index = format!("espipe-bench-{}", unique_id());
    let output = format!("http://localhost:9200/{index}");

    let start = Instant::now();
    let status = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&fixture)
        .arg(&output)
        .arg("-q")
        .status()
        .unwrap();
    assert!(status.success());
    let elapsed = start.elapsed().as_secs_f64();

    let count_output = Command::new("curl")
        .args(["-sS", &format!("http://localhost:9200/{index}/_count")])
        .output()
        .unwrap();
    assert!(count_output.status.success());
    let count_json: Value = serde_json::from_slice(&count_output.stdout).unwrap();
    assert_eq!(count_json["count"].as_u64().unwrap(), BENCH_DOCS as u64);

    println!(
        "fixture={} bytes={} docs={} elapsed_seconds={:.3}",
        fixture.display(),
        fixture_size,
        BENCH_DOCS,
        elapsed
    );
}

#[test]
#[ignore = "requires localhost:9200 and is intended for manual benchmarking"]
fn localhost_nginx_access_log_benchmark() {
    let fixture = nginx_access_benchmark_fixture().unwrap();
    let fixture_size = fs::metadata(&fixture).unwrap().len();
    assert!(fixture_size >= NGINX_BENCH_BYTES_MIN);

    let index = format!("espipe-nginx-bench-{}", unique_id());
    let output = format!("http://localhost:9200/{index}");

    let start = Instant::now();
    let status = Command::new(env!("CARGO_BIN_EXE_espipe"))
        .arg(&fixture)
        .arg(&output)
        .arg("-q")
        .status()
        .unwrap();
    assert!(status.success());
    let elapsed = start.elapsed().as_secs_f64();

    let count_output = Command::new("curl")
        .args(["-sS", &format!("http://localhost:9200/{index}/_count")])
        .output()
        .unwrap();
    assert!(count_output.status.success());
    let count_json: Value = serde_json::from_slice(&count_output.stdout).unwrap();
    assert_eq!(count_json["count"].as_u64().unwrap(), NGINX_BENCH_DOCS as u64);

    println!(
        "fixture={} bytes={} docs={} elapsed_seconds={:.3} type=nginx_access",
        fixture.display(),
        fixture_size,
        NGINX_BENCH_DOCS,
        elapsed
    );
}

fn benchmark_fixture() -> std::io::Result<PathBuf> {
    let path = std::env::var("ESPIPE_BENCH_INPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("espipe-bench-525k.ndjson"));
    if path.is_file() && fs::metadata(&path)?.len() >= BENCH_BYTES_MIN {
        return Ok(path);
    }

    let mut writer = File::create(&path)?;
    for i in 1..=BENCH_DOCS {
        writeln!(
            writer,
            "{{\"id\":{},\"group\":{},\"ok\":true,\"msg\":\"{}\",\"meta\":{{\"source\":\"bench\",\"bucket\":{}}}}}",
            i,
            i % 10,
            "x".repeat(180),
            i % 100
        )?;
    }
    Ok(path)
}

fn nginx_access_benchmark_fixture() -> std::io::Result<PathBuf> {
    let path = std::env::var("ESPIPE_BENCH_NGINX_INPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("espipe-bench-nginx-10m.ndjson"));
    if path.is_file() && fs::metadata(&path)?.len() >= NGINX_BENCH_BYTES_MIN {
        return Ok(path);
    }

    let mut writer = File::create(&path)?;
    for i in 0..NGINX_BENCH_DOCS {
        writer.write_all(nginx_access_log_line(i).as_bytes())?;
        writer.write_all(b"\n")?;
    }
    Ok(path)
}

fn nginx_access_log_line(i: usize) -> String {
    let method = match i % 5 {
        0 => "GET",
        1 => "POST",
        2 => "PUT",
        3 => "PATCH",
        _ => "DELETE",
    };
    let status = match i % 10 {
        0..=5 => 200,
        6 => 201,
        7 => 304,
        8 => 404,
        _ => 500,
    };
    let upstream_status = if status >= 500 { 502 } else { 200 };
    let scheme = if i % 20 == 0 { "http" } else { "https" };
    let host = format!("api-{}.example.internal", i % 64);
    let path = format!("/v1/accounts/{}/orders/{}", i % 50_000, i);
    let query = format!("region={}&limit={}", i % 12, 25 + (i % 200));
    let remote_addr = format!("10.{}.{}.{}", (i / 65_536) % 256, (i / 256) % 256, i % 256);
    let upstream_addr = format!("172.16.{}.{}:8080", (i / 256) % 256, i % 256);
    let user_agent = match i % 4 {
        0 => "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36 Chrome/125.0 Safari/537.36",
        1 => "curl/8.7.1",
        2 => "k6/0.49.0",
        _ => "Datadog-Synthetics/1.0",
    };
    let referer = if i % 3 == 0 {
        format!("https://app.example.internal/dashboard/{}", i % 5000)
    } else {
        "-".to_string()
    };
    let request_length = 512 + (i % 4096);
    let body_bytes_sent = 2048 + ((i * 37) % 16384);
    let request_time = 0.001 + ((i % 4000) as f64 / 1000.0);
    let upstream_time = 0.001 + ((i % 2500) as f64 / 1000.0);
    let trace_id = format!("{:032x}", (i as u128) * 104729 + 17);
    let span_id = format!("{:016x}", (i as u64) * 8191 + 11);
    let ts = 1_710_000_000i64 + i as i64;

    format!(
        "{{\"@timestamp\":{ts},\"service\":\"nginx-gateway\",\"env\":\"bench\",\"host\":\"{host}\",\"scheme\":\"{scheme}\",\"remote_addr\":\"{remote_addr}\",\"request_method\":\"{method}\",\"request_path\":\"{path}\",\"query_string\":\"{query}\",\"status\":{status},\"upstream_status\":{upstream_status},\"request_length\":{request_length},\"body_bytes_sent\":{body_bytes_sent},\"request_time\":{request_time:.3},\"upstream_response_time\":{upstream_time:.3},\"upstream_addr\":\"{upstream_addr}\",\"http_referer\":\"{referer}\",\"http_user_agent\":\"{user_agent}\",\"trace_id\":\"{trace_id}\",\"span_id\":\"{span_id}\",\"geo\":{{\"country\":\"US\",\"region\":\"us-west-2\",\"city\":\"Seattle\"}},\"tls\":{{\"version\":\"TLSv1.3\",\"cipher\":\"TLS_AES_256_GCM_SHA384\"}},\"labels\":{{\"cluster\":\"bench\",\"namespace\":\"edge\",\"pod\":\"nginx-{pod}\"}}}}",
        pod = i % 2048,
    )
}

fn unique_id() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

#[test]
fn generated_fixture_is_large_enough() {
    let fixture = benchmark_fixture().unwrap();
    let bytes = fs::metadata(&fixture).unwrap().len();
    assert!(bytes >= BENCH_BYTES_MIN);

    let line_count = BufReader::new(File::open(fixture).unwrap()).lines().count();
    assert_eq!(line_count, BENCH_DOCS);
}

#[test]
fn generated_nginx_access_line_is_valid_json() {
    let line = nginx_access_log_line(42);
    let value: Value = serde_json::from_str(&line).unwrap();

    assert_eq!(value["service"], "nginx-gateway");
    assert_eq!(value["env"], "bench");
    assert!(value["request_path"].as_str().unwrap().starts_with("/v1/accounts/"));
    assert!(value["http_user_agent"].as_str().unwrap().len() > 5);
    assert!(value["status"].as_i64().unwrap() >= 200);
}
