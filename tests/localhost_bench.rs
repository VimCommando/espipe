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
