## Why

Toon is a compact, line-oriented representation of JSON-like data that can be useful for LLM-generated or human-authored bulk input. `espipe` already streams CSV and NDJSON into the RawValue pipeline; adding Toon input lets users pipe Toon documents without first converting the whole input through an external tool.

## What Changes

- Accept local `.toon` files as structured document inputs.
- Accept remote HTTPS `.toon` inputs when the URL extension or response metadata identifies Toon content.
- Parse Toon input as a stream of documents using the crates.io `toon-format` parser so large inputs do not require full-file materialization.
- Convert each Toon document into a JSON object payload compatible with the existing `Box<RawValue>` ingest and output pipeline.
- Reject Toon inputs that cannot be parsed as object documents with diagnostics that identify the input and document position when available.

## Capabilities

### New Capabilities
- `toon-input`: Local and remote Toon inputs stream into the existing document ingest pipeline.

### Modified Capabilities
- `https-remote-input`: Remote input format detection accepts Toon URLs and Toon response metadata.
- `file-document-import`: Local file import recognizes `.toon` as a structured multi-document input format.
- `rawvalue-document-pipeline`: Toon input produces owned raw JSON documents without forcing steady-state output buffering through `serde_json::Value`.

## Impact

- `src/input.rs`: input-kind detection, local file opening, remote fetch handling, streaming Toon document reader, diagnostics, and tests.
- `Cargo.toml` and `Cargo.lock`: add a `toon-format` dependency from crates.io with default features disabled.
- Tests and fixtures: local `.toon`, remote `.toon`, malformed Toon, and document-shape coverage.
- No CLI flag changes are expected; format selection remains extension and metadata based.
