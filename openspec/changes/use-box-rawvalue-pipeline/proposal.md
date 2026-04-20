## Why

`espipe` currently parses each NDJSON line into `serde_json::Value`, clones that `Value` into the Elasticsearch queue, and re-serializes it again for file and stdout outputs. That allocation-heavy path leaves avoidable memory pressure and throughput on the table for large imports.

The `esdiag` change in elastic/esdiag#285 showed the same general optimization is practical: keep pass-through JSON as `Box<RawValue>` for as long as possible, only materializing structured values where mutation is unavoidable. `espipe` is an even better fit because it does not transform documents.

## What Changes

- Replace the document type carried through the input and output pipeline from `serde_json::Value` to `Box<serde_json::value::RawValue>`.
- Make `Input::read_line` produce raw JSON payloads for NDJSON input and raw JSON strings for CSV-derived documents.
- Change `Output::send` and sender implementations to consume owned raw documents instead of borrowing `Value`.
- Introduce a bounded async handoff between document production and Elasticsearch batch flushing so input parsing does not directly own network pacing.
- Update file and stdout outputs to write raw JSON directly without reparsing or re-serializing.
- Update the Elasticsearch bulk sender to preserve raw documents in the queue and emit valid `_bulk` NDJSON without routing documents through `Value` on the hot path.
- Allow a lightweight metadata envelope if it simplifies channel payloads or future output accounting without reintroducing `Value` in the hot path.
- Add performance coverage for a large-input benchmark against `localhost:9200`, including documented before/after results and document-count parity checks.

## Capabilities

### New Capabilities
- `rawvalue-document-pipeline`: Carry pass-through JSON documents as `Box<RawValue>` through ingest and output paths while preserving import correctness and benchmark visibility.

### Modified Capabilities

None.

## Impact

- Affected code: [src/input.rs](/Users/reno/Development/espipe/src/input.rs), [src/output.rs](/Users/reno/Development/espipe/src/output.rs), [src/output/file.rs](/Users/reno/Development/espipe/src/output/file.rs), [src/output/elasticsearch.rs](/Users/reno/Development/espipe/src/output/elasticsearch.rs), [src/main.rs](/Users/reno/Development/espipe/src/main.rs), [Cargo.toml](/Users/reno/Development/espipe/Cargo.toml)
- Dependency impact: enable `serde_json`'s `raw_value` feature
- Runtime impact: lower allocation churn in the hot read/queue path; expected improvement is lower steady-state memory and better ingest throughput for large NDJSON imports
- Benchmark note: the implemented change was validated on a `136,861,395`-byte fixture (`525,000` docs) against `localhost:9200` at the target `5,000`-document batch size. The baseline `HEAD` build completed in `1.47s` real time with `94,076,928` bytes max RSS, while the `Box<RawValue>` implementation completed in `1.34s` real time with `75,644,928` bytes max RSS. Both runs reached full `525,000`-doc parity. The baseline also emitted a spurious `400 parse_exception` from an empty final bulk flush, which this change removes.
