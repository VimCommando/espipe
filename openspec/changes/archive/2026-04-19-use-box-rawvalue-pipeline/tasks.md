## 1. RawValue Pipeline

- [x] 1.1 Enable `serde_json`'s `raw_value` feature and replace the shared document type in the input/output pipeline with `Box<serde_json::value::RawValue>`
- [x] 1.2 Update `Input::read_line`, the main ingest loop, and sender trait signatures to move owned raw documents instead of borrowing `Value`
- [x] 1.3 Convert CSV input to produce raw JSON objects without reparsing through `Value`

## 2. Output Implementations

- [x] 2.1 Update stdout and file outputs to write raw JSON bytes directly
- [x] 2.2 Rework Elasticsearch output to use a bounded async channel, store `Box<RawValue>` in the worker queue, and emit valid `_bulk` NDJSON bodies without `Value` in the queue while preserving a target batch size of `5,000`
- [x] 2.3 Decide whether a lightweight metadata envelope is needed for the worker contract and, if used, keep it separate from the raw body
- [x] 2.4 Add regression coverage for NDJSON, CSV, file/stdout parity, Elasticsearch bulk request correctness, and backpressure/in-flight limits

## 3. Benchmarking And Validation

- [x] 3.1 Create or document a reproducible localhost benchmark fixture of at least 100 MB and validate full document-count parity against `localhost:9200`
- [x] 3.2 Add a checked-in benchmark target in the test suite for the localhost large-file ingest path
- [x] 3.3 Run before/after benchmarks for the current implementation and the `Box<RawValue>` implementation, recording elapsed time, throughput, and any relevant memory observations
- [x] 3.4 Document any benchmark harness caveats discovered during implementation, including whether bulk backpressure or flush pacing changes were required to keep the `5,000`-document target batch size stable
