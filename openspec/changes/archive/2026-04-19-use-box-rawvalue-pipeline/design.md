## Context

`espipe` is a pass-through bulk loader. For NDJSON input it does not need to inspect or mutate document bodies, but the current implementation still deserializes every line into `serde_json::Value`, clones queued documents, and serializes them again on output. The Elasticsearch output keeps a `Vec<Value>` queue and turns that into bulk operations, which makes large imports pay for repeated allocations even though document structure is effectively opaque.

The upstream pattern from elastic/esdiag#285 is relevant here: keep JSON payloads in `RawValue` form until a code path truly needs structured access. The local spike for this proposal confirmed the general direction is sound, but it also exposed a transport detail: `BulkOperation<Box<RawValue>>` produced `400 parse_exception` responses against the local `localhost:9200` Elasticsearch 9.3.0-SNAPSHOT container, so the Elasticsearch sink needs an explicit NDJSON emission path rather than relying on the generic bulk serializer.

Reviewing `esdiag`'s `StreamingDocumentExporter` pattern adds one more useful constraint: the important reuse for `espipe` is not the streaming data-source trait or lookup fan-out, but the bounded async channel between document production and export workers. That separation improves resilience when the output side slows down and avoids the current design where input parsing and bulk flushing are tightly coupled in the same loop.

Benchmark fixture used during implementation:

- Input: `/tmp/espipe-bench-525k.ndjson`
- Size: `136,861,395` bytes
- Docs: `525,000`
- Target: `http://localhost:9200`
- Execution path: direct CLI ingest to `localhost:9200` with a target bulk batch size of `5,000`

Observed implementation benchmark:

| Variant | Real Time | Throughput | Count |
| --- | ---: | ---: | ---: |
| Current `Value` pipeline (`HEAD`) | `1.47s` | `357,143 docs/s` (`88.79 MiB/s`) | `525,000` |
| `Box<RawValue>` implementation | `1.34s` | `391,791 docs/s` (`97.40 MiB/s`) | `525,000` |

Observed memory footprint from `/usr/bin/time -l`:

| Variant | Max RSS |
| --- | ---: |
| Current `Value` pipeline (`HEAD`) | `94,076,928` bytes |
| `Box<RawValue>` implementation | `75,644,928` bytes |

## Goals / Non-Goals

**Goals:**

- Remove `serde_json::Value` from the normal read, handoff, and queue path for pass-through documents.
- Preserve CLI behavior and document correctness for NDJSON, CSV, stdout, file, and Elasticsearch outputs.
- Avoid cloning full document trees when buffering Elasticsearch bulk requests.
- Add a repeatable localhost benchmark that captures both throughput and document-count parity for a fixture of at least 100 MB.

**Non-Goals:**

- Changing CLI flags, authentication behavior, or URI parsing.
- Introducing document transformation features.
- Solving every existing bulk transport issue unrelated to document representation.
- Requiring a specific Elasticsearch version beyond what the project already targets.

## Decisions

### Use `Box<RawValue>` as the pipeline document type

`Input::read_line` and `Output::send` will exchange owned `Box<RawValue>` values.

Rationale:

- `RawValue` stores the original JSON text and avoids allocating a full `Value` tree.
- Ownership through `send` removes the current clone in `ElasticsearchOutput::send`.
- `espipe` is pass-through oriented, so opaque JSON is the correct default representation.

Alternatives considered:

- Keep `Value` and optimize queueing only: rejected because the parse and clone cost still occurs on every document.
- Use `&RawValue` everywhere: rejected because the current loop reuses the line buffer, so borrowed payloads would be invalid once the next line is read.

### Generate raw JSON for CSV rows once

CSV rows will still become JSON objects, but the result will be converted directly into `Box<RawValue>` rather than `Value`.

Rationale:

- CSV input still needs object construction, but it does not need a second parse into `Value`.
- This keeps the document type uniform across all outputs.

Alternatives considered:

- Leave CSV on `Value` while NDJSON uses `RawValue`: rejected because it complicates the sender interface and weakens the performance story.

### Write file and stdout outputs directly from raw bytes

File and stdout outputs will write `value.get()` directly plus a trailing newline.

Rationale:

- These outputs do not need structural access.
- This removes unnecessary `serde_json::to_writer` work.

Alternatives considered:

- Re-serialize `RawValue` through `serde_json`: rejected because it gives back the work this change is meant to remove.

### Emit Elasticsearch `_bulk` request bodies explicitly

The Elasticsearch sink should maintain `Vec<Box<RawValue>>` in memory, then build NDJSON bulk request bytes from action lines plus raw document bytes when flushing.

Rationale:

- The spike showed `BulkOperation<Box<RawValue>>` is not reliable on the local Elasticsearch 9.3.0-SNAPSHOT path.
- Manual NDJSON emission avoids reparsing documents into `Value` and preserves the intended memory benefit.
- The `_bulk` format is simple and stable for `create` operations.

Alternatives considered:

- Convert `RawValue` back to `Value` at flush time and keep using `BulkOperation`: acceptable as a temporary spike fallback, but rejected for the final design because it reintroduces parse cost on the send path.
- Keep current `BulkOperation<Value>` implementation: rejected because it defeats the main purpose of the change.

### Add a bounded output worker for Elasticsearch sends

`espipe` should add a dedicated async output worker for Elasticsearch that receives owned documents over a bounded `tokio::sync::mpsc` channel, batches them with a target batch size of `5,000`, and flushes them with an explicit in-flight limit.

Rationale:

- This borrows the strongest part of `esdiag`'s exporter pattern: producer and network sink progress independently, with backpressure expressed by the bounded channel.
- It directly addresses the benchmark behavior seen during proposal work, where overly eager flush task creation could outrun the local Elasticsearch container.
- `espipe` remains simple because it only needs one channel and one output worker, not the full `StreamingDataSource` and multi-stream processor hierarchy.

Alternatives considered:

- Keep the current direct `send` path and only change the queued document type: rejected because transport pacing remains coupled to the read loop.
- Port `StreamingDocumentExporter` wholesale: rejected because `espipe` does not need streaming source abstractions, per-type fan-out, or lookup context.

### Keep metadata lightweight and separate from the raw body

The design may introduce a small metadata wrapper around each queued document in the first implementation if it simplifies batching, counters, or output accounting, but metadata must not require reparsing document bodies into `Value`.

Rationale:

- The user explicitly called out metadata as acceptable.
- A wrapper can carry bookkeeping such as source line count or future document attributes without changing the raw body representation.
- `esdiag` already demonstrates that metadata can be carried separately from large passthrough payloads.

Alternatives considered:

- Ban metadata entirely: rejected because a small envelope may make the worker interface cleaner.
- Always attach a serialized metadata object to every document: rejected because that broadens scope and risks overhead without a concrete need.

### Benchmark on localhost with an explicit reproducibility contract

The change will include a documented benchmark procedure that uses:

- a fixture of at least 100 MB
- `localhost:9200`
- before/after wall time
- document-count parity checks
- a checked-in benchmark target in the test suite to make the measurement procedure repeatable

Rationale:

- The request for this change is performance-motivated, so the spec needs a measurable acceptance check.
- Reproducibility matters more than one-off numbers.
- A checked-in benchmark keeps the large-file measurement close to the code that is being optimized.

Alternatives considered:

- Rely on synthetic unit benchmarks only: rejected because the user asked for end-to-end localhost results.

## Risks / Trade-offs

- [Elasticsearch client serialization path may not support `RawValue` directly] → Build raw `_bulk` NDJSON request bodies explicitly and cover them with integration tests against `localhost:9200`.
- [Unbounded send concurrency can erase throughput gains or trigger cluster-side failures] → Use a bounded channel plus explicit in-flight request limits in the Elasticsearch output worker.
- [Malformed NDJSON lines become opaque strings until output time] → Keep JSON validation in `Input::read_line` by parsing each line as `Box<RawValue>` rather than blindly wrapping strings.
- [CSV still allocates one JSON string per record] → Accept as a bounded cost because CSV must be rendered as JSON objects anyway.
- [Benchmark comparisons may be confused by the baseline's empty final bulk flush bug] → Record actual document-count parity and note that the baseline's spurious `400 parse_exception` comes from sending an empty final `_bulk` request after all documents were already flushed.
- [Owning documents in `send` changes trait signatures] → Update all senders in one change and add compile-time coverage via tests/build.

## Migration Plan

1. Enable `serde_json`'s `raw_value` feature and update the shared document type and sender trait signatures.
2. Convert NDJSON and CSV inputs to produce `Box<RawValue>`.
3. Update stdout and file outputs to consume raw documents directly.
4. Rework Elasticsearch output to hand off owned documents over a bounded async channel, buffer `Box<RawValue>` in the worker, and flush explicit `_bulk` NDJSON bytes with controlled in-flight concurrency.
5. Add regression tests for document parity and a checked-in benchmark target in the test suite for localhost large-file measurements.
6. Run before/after localhost benchmarks on a fixture of at least 100 MB and record the results in the change artifacts.

Rollback:

- Revert the sender/document-type changes and restore `Value`-based buffering if correctness or compatibility regressions appear.

## Implementation Constraints

- The final implementation SHALL target a `BATCH_SIZE` of `5,000`; if localhost transport issues persist at that size, the fix belongs in backpressure and flush pacing rather than lowering the batch target.
- Metadata is acceptable in the first implementation only as a lightweight wrapper that stays separate from the raw document body.
- The benchmark SHALL be exposed as a checked-in benchmark in the test suite rather than only as an ad hoc command sequence.
