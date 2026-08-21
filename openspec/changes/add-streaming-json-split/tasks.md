## 1. Split option and path parsing

- [x] 1.1 Add `--split <json_pointer>`, pass the optional path into input construction, and reject split mode with more than one input before opening the output.
- [x] 1.2 Parse split paths into decoded tokens, implementing `/` as the root alias, optional non-root trailing slash normalization, RFC 6901 `~1`/`~0` decoding, and actionable validation errors.
- [x] 1.3 Route local paths, `file://` URIs, stdin, and HTTP/HTTPS JSON sources through split parsing when selected while preserving all existing input routes when the option is absent.

## 2. Incremental pointer navigation

- [x] 2.1 Add navigation `DeserializeSeed`/`Visitor` types that traverse object tokens with `MapAccess`, discard unmatched values with `IgnoredAny`, and report missing keys with full split-path context.
- [x] 2.2 Add array-token navigation with `SeqAccess`, canonical zero-based index validation, bounded skipping before and after the selected element, and contextual missing/invalid-index errors.
- [x] 2.3 Invoke the terminal collection visitor at the resolved value and validate the remainder of enclosing containers plus `Deserializer::end()` so malformed suffixes and trailing JSON are detected.

## 3. Streaming collection handoff

- [x] 3.1 Add a split `Input` variant and bounded document/failure/completion handoff whose blocking parser worker owns the source reader and exits when the consumer disconnects.
- [x] 3.2 Stream a terminal map incrementally, materializing one value at a time, requiring an object, rejecting an existing `id`, inserting the property key as a string `id`, and producing `Box<RawValue>`.
- [x] 3.3 Stream a terminal array incrementally, materializing one element at a time, requiring an object, preserving its fields without a synthetic ID, and producing `Box<RawValue>`.
- [x] 3.4 Preserve applicable origin metadata and propagate source-, path-, key-, index-, and parse-location-aware failures through `Input::read_next`, including clean empty-collection completion.

## 4. Correctness and bounded-streaming coverage

- [x] 4.1 Add checked-in root and nested map/array fixtures and tests for map-key string IDs, unchanged array objects, nested value preservation, and empty collections.
- [x] 4.2 Add split-path tests for `/`, equivalent `/hits` and `/hits/`, escaped `~0`/`~1` keys, intermediate array indices, malformed pointers, missing keys/indices, and scalar traversal.
- [x] 4.3 Add child-shape failure tests for scalar/null/nested-array children, selected scalar/null values, map `id` conflicts, malformed JSON, and trailing JSON with contextual diagnostics.
- [x] 4.4 Add an instrumented reader and bounded-handoff test proving the first batch is available before EOF and parsing waits rather than accumulating the complete collection when the consumer is gated.
- [x] 4.5 Add generated maps and arrays larger than an Elasticsearch batch and verify document-count, ID, and value parity without a platform-specific RSS assertion.
- [x] 4.6 Add CLI regressions for stdout and NDJSON file output, multiple-input rejection, remote origin metadata, and unchanged `.json`, `.ndjson`, and stdin behavior without `--split`.
- [x] 4.7 Extend Elasticsearch integration coverage to verify split documents reuse bulk action/configuration, ingest-pipeline, and index-template paths.

## 5. Documentation and verification

- [x] 5.1 Update the README CLI reference, supported inputs, pointer normalization/escaping, array-versus-map behavior, errors, examples, and one-document memory-bound limitation for `--split`.
- [x] 5.2 Add root-object and wrapped-array JSON commands beside the CSV command in `examples/steam-games/readme.md`, including map-key `id` behavior.
- [x] 5.3 Run Rust formatting, linting, the complete test suite, and focused CLI/integration tests; confirm existing input/output behavior remains unchanged.

## 6. Unordered parallel split throughput

- [x] 6.1 Revise the split contract and documentation to state that output order is unspecified and throughput takes priority over resequencing.
- [x] 6.2 Capture selected children as raw JSON into bounded batches so the parser can continue while CPU workers transform prior batches.
- [x] 6.3 Process batches concurrently using available parallelism, publish completed batches without restoring source order, and propagate cancellation/failures safely.
- [x] 6.4 Adapt `Input::read_next` to drain completed batches while retaining its one-document interface and existing output behavior.
- [x] 6.5 Replace order-sensitive tests with exact document-set parity and add coverage for bounded batch streaming, invalid children, and collections spanning many worker batches.
- [x] 6.6 Document parallel batching, run formatting/lint/full tests, and benchmark the large local JSON input against a local file output without persisting its path.
- [x] 6.7 Benchmark the large local JSON input against localhost Elasticsearch and verify the indexed document count.

## 7. Verification fixes

- [x] 7.1 Preserve arbitrary-precision JSON numbers through map and array split transformation and add regression coverage.
- [x] 7.2 Reject double trailing slashes that would address a final empty-name member, then document and test the path rule.
- [x] 7.3 Strengthen coverage for consumer backpressure, exact large-collection parity, late errors after emitted batches, missing array indices, and scalar traversal.
- [x] 7.4 Make the localhost Elasticsearch integration test explicitly opt-in so an unavailable node cannot produce a false passing result.
- [x] 7.5 Run formatting, the full test suite, linting, strict OpenSpec validation, and focused regression commands.
