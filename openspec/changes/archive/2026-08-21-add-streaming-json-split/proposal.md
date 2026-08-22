## Why

Large JSON inputs are often one array or object, sometimes nested beneath response wrappers, and cannot be ingested as NDJSON without first materializing and reshaping the payload. `espipe` needs a general way to select a collection and stream its children as documents while preserving bounded memory and the existing output pipeline.

## What Changes

- Add an explicit `--split <json_pointer>` input mode for a single JSON source.
- Use `/` as an ergonomic root split and accept an optional trailing slash, so `/hits` and `/hits/` both select the collection beneath `hits`.
- Resolve nested pointer tokens incrementally, including RFC 6901 `~0` and `~1` escapes and numeric array indices, without materializing skipped wrappers or the selected collection.
- Stream both selected JSON arrays and selected JSON objects through bounded, parallel batches; output order is intentionally unspecified to maximize throughput.
- Emit each array element as one document. Emit each object property value as one document with the property name added as its string `id`, rejecting conflicts instead of silently overwriting data.
- Feed emitted documents through the existing stdout, file, and Elasticsearch output paths, including bulk batching, ingest pipelines, and index templates.
- Preserve existing NDJSON, ordinary `.json`, stdin, CSV, Toon, remote-input, glob, and multi-file behavior when `--split` is not supplied.
- Add fixtures and coverage for root and nested arrays/maps, pointer resolution, malformed inputs, bounded streaming, and output integration.
- Document split JSON ingestion alongside the Steam Games CSV example.

## Capabilities

### New Capabilities

- `json-split-input`: Explicit pointer-selected JSON collection streaming, array/map document conversion, map-key identifiers, validation, errors, and existing-output integration.

### Modified Capabilities

None.

## Impact

- CLI: `src/main.rs` gains `--split <json_pointer>` plus single-input validation.
- Input pipeline: `src/input.rs` gains an incremental pointer navigator and collection reader built on `serde_json::Deserializer`, `DeserializeSeed`, `Visitor`, `MapAccess`, and `SeqAccess`, adapting the approach introduced in `toon-rust` commit `9c7007a358e25a0c453fd54e02e287ac465ea824`. Bounded batches follow esdiag's streaming-data-source pattern and are transformed by a parallel worker pool before entering the existing output pipeline.
- Tests and fixtures: input unit tests and CLI/output integration tests gain representative and large synthetic arrays/maps, nested wrappers, pointer errors, malformed JSON, and conflicting IDs.
- Documentation: `README.md` and `examples/steam-games/readme.md` gain split syntax, examples, and format rules.
- Dependencies: no new runtime crate is expected; the implementation uses the existing `serde`/`serde_json` stack.
