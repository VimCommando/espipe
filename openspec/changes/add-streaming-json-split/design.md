## Context

See `proposal.md` for motivation. The main ingest loop pulls one owned `Box<serde_json::value::RawValue>` at a time from `Input::read_next` and awaits `Output::send`; Elasticsearch output already provides bounded batching and concurrency. Local and downloaded JSON currently use a line reader, with a special case that reads a pretty-printed object beginning with a line containing only `{` into memory. Automatically treating a root object or array as a collection would change existing single-document JSON behavior, so splitting must remain explicit.

The useful pattern from `toon-rust` commit `9c7007a358e25a0c453fd54e02e287ac465ea824` is a `serde_json::Deserializer` driven by `DeserializeSeed` and `Visitor`: `MapAccess` and `SeqAccess` allow selected structure to be consumed incrementally while only individual document subtrees are materialized. Unlike the Toon encoder, `espipe` must navigate to an arbitrary selected collection and yield each child to an async consumer.

## Goals / Non-Goals

**Goals:**

- Select a root or nested collection with predictable JSON Pointer-compatible syntax.
- Stream selected arrays and maps with memory proportional to bounded batches plus existing bounded output buffers.
- Maximize throughput with parallel document transformation and explicitly avoid the cost of restoring source order.
- Preserve the pull-like `Input::read_next` contract and owned raw-document output contract.
- Preserve a selected map's keys as authoritative document IDs without silently destroying an existing `id`.
- Propagate pointer, parser, and validation failures after any previously produced documents, consistent with streaming NDJSON behavior.
- Support the single input forms that can already provide JSON bytes: local paths and `file://` URIs, stdin, and HTTP/HTTPS JSON sources.

**Non-Goals:**

- Automatically infer a split path from a suffix or payload shape.
- Select multiple paths, use JSONPath filters/wildcards, recursively flatten descendants, or retain dropped wrapper fields.
- Add synthetic IDs to array elements or make the map identifier field configurable.
- Add compressed JSON suffixes beyond combinations already supported by the repository.
- Make streaming ingestion transactional or roll back documents sent before a later parse error.
- Avoid materializing an individual emitted document.

## Decisions

### Use one explicit JSON Pointer-derived option

Add `--split <json_pointer>` and thread the optional value into `Input::try_new`. Split mode requires exactly one input. It treats that source as one JSON document independently of line boundaries; without the option, ordinary `.json`, `.ndjson`, stdin, and remote JSON retain current behavior.

RFC 6901 defines the empty string as the root pointer and `/` as a member with an empty name. For command-line ergonomics and the requested examples, split mode defines `/` as a root alias and ignores one trailing slash on non-root paths, making `/hits` and `/hits/` equivalent. All remaining tokens follow RFC 6901 rules: decode `~1` before `~0`, compare object keys without Unicode normalization, and treat a canonical decimal token as an array index while traversing an array. The trade-off is that split mode cannot address a final empty-name member; document this intentional deviation and reject malformed escapes and non-absolute paths during CLI validation.

Automatic sniffing was rejected because an intentional one-document object/array and a collection share the same root shapes. A generic input-format enum was rejected because `--split` describes a transformation orthogonal to file extension and can coexist with a future format override.

### Compile the pointer and navigate incrementally with Serde seeds

Parse the CLI value once into decoded reference tokens. A navigation `DeserializeSeed` consumes one token at a time:

- On an object, iterate `MapAccess`, send the matching value through the next navigation seed, and consume unmatched values with `IgnoredAny`.
- On an array, validate the token as a canonical zero-based index, consume preceding and following elements with `IgnoredAny`, and send the matching element through the next navigation seed.
- At the terminal target, invoke a collection visitor rather than deserialize a `Value`.

Missing keys/indices, traversal through scalars, and a terminal scalar/null become contextual split-path errors. Consuming siblings after the selected value and calling `Deserializer::end()` ensures malformed suffixes or trailing root values are still detected. This approach reads but does not materialize skipped wrapper branches.

Deserializing the root into `Value` and applying `Value::pointer` was rejected because it defeats bounded memory. A custom JSON token parser was rejected because Serde already provides correct syntax, escaping, and line/column diagnostics. JSONPath was rejected because filters and multiple matches introduce broader semantics and buffering questions not needed here.

### Split the terminal collection into bounded raw batches

The terminal visitor accepts `MapAccess` or `SeqAccess` and captures each child as `Box<RawValue>` so the single JSON reader only finds document boundaries and validates the enclosing JSON. It groups children into small bounded batches:

- For a map, retain each property name with its raw value. A CPU worker requires an object, rejects a pre-existing `id`, inserts `id: Value::String(property_name)`, then serializes it into `Box<RawValue>`.
- For an array, retain each zero-based index with its raw value. A CPU worker requires an object and serializes it compactly without adding an ID.

Workers use Serde's arbitrary-precision number representation when materializing documents, so compact serialization does not round large integers or high-precision decimals. Nested objects and arrays inside an emitted document are preserved. Empty selected arrays/maps complete with no documents. A selected non-collection and non-object children fail rather than being wrapped because all downstream documents must remain JSON objects.

Map keys become IDs to retain issue #14's catalogue semantics. Array indices are positional rather than domain identifiers, so synthesizing them as IDs was rejected. Rejecting an existing map-value `id` was chosen over overwrite/preserve behavior because either silently makes one identifier non-authoritative.

### Bridge Serde's visitor lifetime with bounded, unordered parallel batches

Create a split-input variant backed by two bounded `std::sync::mpsc::sync_channel` stages. A blocking parser worker owns the `Read + Send` source and sends raw document batches into a worker pool sized from available parallelism. Workers validate and transform batches concurrently, then send completed document batches to `Input::read_next`. The input variant drains its current batch before receiving another, preserving the existing one-document pull contract at the output boundary.

The result channel does not resequence completed batches. Documents within a batch retain their local order, but batches can complete in any order, so split mode makes no output-order guarantee for maps or arrays. This is deliberate: restoring source order would introduce a serial coordination point and retain completed batches behind a slower predecessor.

A worker boundary is necessary because `MapAccess` and `SeqAccess` are scoped to one visitor call and cannot be stored between repeated `read_next` calls. Both queues apply backpressure, bounding retained raw and transformed documents independently of collection length. If the consumer disconnects or any worker reports an invalid child, shared cancellation stops new work; already-running batches may finish before cancellation is observed.

### Reuse source preparation and output dispatch

Split mode reuses existing local readers, remote download/temp-file handling, URI origin metadata, and the main output loop. Remote JSON split mode bypasses only NDJSON-shape validation that conflicts with a single wrapped document. Each emitted child receives existing origin metadata where that source type already supplies it, then enters the same stdout, file, or Elasticsearch output.

Split worker batches are fixed internal scheduling units rather than collection-sized buffers. Existing `--batch-size` and `--max-requests` settings independently continue to define Elasticsearch bulk buffering, pipeline execution, and request concurrency.

### Prove streaming with gated and generated inputs

Check in small fixtures for root/nested arrays and maps plus invalid shapes. Add instrumented/gated reader tests proving the first bounded batch crosses the handoff before EOF and that bounded channels stop further reading when the consumer pauses. Generated inputs larger than a bulk batch cover unordered count and ID/value parity without committing a production dataset.

This deterministic evidence is preferred to an RSS threshold, which is platform-sensitive and unreliable in normal CI. Include pointer escaping, intermediate array traversal, missing paths, malformed suffixes, and unchanged no-flag behavior in focused tests.

## Risks / Trade-offs

- [The root/trailing-slash conveniences deviate from strict RFC 6901 empty-key semantics] → Document the normalization prominently and test it alongside standard `~0`/`~1` decoding.
- [A single emitted document or skipped sibling can itself be very large] → Document that emitted documents are materialized individually; `IgnoredAny` keeps skipped values out of `Value` trees even though their bytes must be parsed.
- [The parser remains sequential because one JSON byte stream must be tokenized in order] → Keep that stage allocation-light by capturing raw values, then parallelize object materialization, validation, ID insertion, and serialization across all remaining available CPUs.
- [Parallel batches change observable ordering] → Define split order as unspecified and test document-set parity rather than sequence. Users requiring order can include an explicit sortable field in their documents.
- [A worker pool adds threads for split input] → Split mode permits exactly one input, sizes the pool from available parallelism, and uses bounded queues.
- [An output error may leave the worker parsing its current document briefly] → Treat receiver disconnection as cancellation at the next handoff and let the worker-owned reader/temp file drop when it exits.
- [Late malformed content can be discovered after other batches were sent or are in flight] → Preserve streaming semantics, cancel pending work, and report clearly without implying rollback or a strict error boundary.
- [Remote input is downloaded fully to a temporary file before split parsing] → This preserves current HTTP behavior and bounded memory; direct response streaming is a separate transport optimization.

## Migration Plan

1. Add split-path parsing and CLI validation while leaving the default path untouched.
2. Add the pointer navigation seeds, terminal collection visitor, and bounded worker handoff.
3. Route supported existing source readers into split mode and verify output parity.
4. Add fixtures, unit/integration coverage, and documentation before release.

Rollback consists of removing the opt-in option and split input variant; no stored data, configuration, or default behavior requires migration.
