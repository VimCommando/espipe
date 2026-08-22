## Context

The current CLI exposes `create`, `index`, and `update`, defaults to `create`, and constructs bulk bodies directly from `Box<RawValue>` documents. `update` extracts a top-level string `_id`, but the bulk response model only recognizes `create` and `index` items. Local inputs are resolved through several parser-specific paths, so source identity and metadata must be defined independently from document format and parser choice.

The proposal and delta specs define the required user-visible behavior. This design describes how to add it without making non-Elasticsearch outputs emit bulk metadata.

## Goals / Non-Goals

**Goals:**

- Keep Elasticsearch action selection at the bulk NDJSON boundary.
- Make generated local-source IDs independent of content, timestamps, and absolute checkout paths.
- Distinguish single-source and multi-source inputs before parsing documents.
- Apply per-file transformations, including `--split`, consistently across multi-source inputs.
- Preserve raw JSON document bodies for file and stdout outputs.
- Keep document identity metadata separate from the source body until Elasticsearch bulk serialization.
- Extend bulk response handling to recognize update results and no-ops.

**Non-Goals:**

- Synchronize files deleted from the bundle into Elasticsearch.
- Discover or clone remote Git repositories.
- Add a bundle manifest or a user-supplied bundle-ID option.
- Promise any operational behavior after Elasticsearch receives an `update` or `upsert` payload.
- Add generated IDs to non-file input forms.

## Decisions

### Use explicit bulk action mappings

Extend the existing `BulkAction` value enum with `Upsert` and change its default to `Index`. Keep action-specific serialization in the Elasticsearch output layer:

- `create` emits `create` metadata and a source line.
- `index` emits `index` metadata and a source line.
- `update` emits `update` metadata and a payload containing `doc`.
- `upsert` emits `update` metadata and a payload containing `doc` plus `doc_as_upsert: true`.

This keeps Elasticsearch responsible for action semantics. The client only selects the bulk operation, constructs the documented payload, and interprets the bulk response. It does not promise replacement, merging, versioning, or no-op behavior beyond the request and response payloads. File and stdout outputs continue to write raw documents and do not serialize action metadata.

### Resolve source cardinality before parsing

Classify local input after expanding explicit paths and globs:

- A **single-source input** resolves to one physical file. It may emit one or many documents. Generated IDs are disabled by default for ease of use.
- A **multi-source input** resolves to more than one physical file. Generated IDs are enabled by default, independently for each source file.
- An explicit `--generate-id=true` enables generated IDs for either cardinality, while `--generate-id=false` disables them for either cardinality.

The omitted option therefore has an effective `auto` mode. The CLI may continue to expose boolean values, but the input pipeline must preserve the distinction between an omitted value and an explicit `true` or `false`.

`--split` is a per-file transformation. For a multi-source input, each file is opened and split independently, preserving that file's origin and identity namespace. A split invocation is not limited to a single path or glob merely because the split operation itself acts on one file at a time.

### Filter multi-source discovery candidates

Apply discovery policies to multi-source discovery inputs: multiple local positionals and glob patterns. A direct, explicitly named single file remains usable without discovery-policy opt-ins. Apply filtering before source cardinality is calculated, so skipped candidates do not enable the multi-source generated-ID default.

Expose `--symlinks=skip|fail|follow`, defaulting to `skip`. `skip` removes candidates whose path contains a symlink component. `fail` rejects the input when such a candidate is encountered. `follow` keeps the user-supplied lexical path for `origin` and generated-ID identity while allowing the symlink target to be outside the working directory. Direct non-symlink paths outside the working directory remain rejected for multi-source inputs.

Expose `--hidden=skip|fail|include`, defaulting to `skip`. A hidden path has any dot-prefixed component; `skip` removes it, `fail` rejects it, and `include` retains it. If filtering removes every candidate, input resolution fails with no regular files resolved.

### Carry generated identity alongside the raw body

Introduce an internal document envelope containing the existing owned `Box<RawValue>` plus optional transport identity metadata. The input layer sets the generated identity for local file documents when effective generation is enabled. The Elasticsearch output resolves explicit top-level `_id` values, gives them precedence, and removes the transport-only field while building the bulk body. File and stdout outputs ignore the transport identity and continue writing the raw document body.

This avoids injecting generated IDs into user-visible raw output and avoids reparsing documents solely to discover generated file identity. Existing raw JSON buffering remains intact.

### Derive IDs from a canonical source key

For a local file document without an explicit `_id`, construct a canonical UTF-8 key from the bundle identifier, the source file's working-directory-relative path, and, when needed, a typed document discriminator:

```text
<bundle identifier> + "\0" + <working-directory-relative path> + ["\0" + <discriminator>]
```

Normalize the relative path to use `/` separators, remove a leading `./` when the path is inside the working directory, and retain the filename extension. Hash the key with SHA-256, retain the first 16 bytes (128 bits), and encode those bytes with URL-safe Base64 without padding. The resulting 22-character value is the generated Elasticsearch ID. Keep the full canonical key and SHA-256 input unchanged; only the generated ID representation is compacted.

Use a discriminator for every generated document when the source can emit multiple documents. A single-document source uses a fixed `0` discriminator when generation is explicitly enabled. An ordinary record stream uses its zero-based record ordinal. A split object uses the source key, and a split array uses the zero-based array index. Encode the discriminator with its type so an object key and an array index cannot collide. The split key or index is transport identity only and does not need to be inserted into the document body.

Resolve the bundle identifier by locating the tracked Git working-tree root and using its directory name. If no tracked Git working tree is found, use the agreed parent-directory fallback for the working path. Do not include the absolute checkout path in the canonical key.

The same path normalization feeds local `origin.path` and `origin.filename` metadata. Multi-source discovery SHALL reject direct files outside the working directory. Symlink paths that escape the working directory are skipped by default, rejected with `--symlinks=fail`, and allowed with `--symlinks=follow`; the lexical symlink path remains the source identity and origin path. Single-source inputs may reference files outside the working directory; their relative path may contain `..` when an explicit `--generate-id=true` requests an ID.

### Generate IDs for local sources only

Add a top-level `--generate-id=true|false` option with an omitted `auto` mode. Pass the effective setting into every local file-source construction, regardless of whether the parser handles converted documents, structured records, or split documents. Non-file inputs, including stdin and remote inputs, do not receive generated identity from this feature.

When `update` or `upsert` requires an ID and neither an explicit nor generated ID exists, fail while constructing the operation. `create` and `index` omit `_id` in that case and preserve Elasticsearch's automatic ID behavior.

### Generalize local origin metadata

Build local file origins from the working-directory-relative path for every emitted document, including single-source files, multi-source files, and documents produced by `--split`. Use `scheme: file`, the relative containing directory as `path`, and the final component as `filename`; represent a root-level file's directory as `./`.

Update the existing file-document requirement delta from `file.path` and `file.name` to `origin`. Keep origin metadata in the document body because it is source metadata, while keep generated identity in the internal envelope because it is transport metadata.

### Tolerate duplicate Markdown frontmatter keys

Use a frontmatter-specific YAML deserialization path that accepts repeated mapping keys and records them for warnings. If a key occurs more than once, the last value replaces the earlier value, matching the effective mapping behavior expected by document authors. Emit a warning containing the source path and duplicate key, then continue importing the document.

Keep ordinary YAML file imports strict. Duplicate Markdown frontmatter keys are the only tolerated YAML parse condition; non-mapping frontmatter, malformed YAML, and conflicts with the configured content field remain fatal input errors.

### Extend bulk response parsing

Add an `update` variant to the bulk response item model and treat successful update statuses, including `result: "noop"`, as successful counts. Preserve existing logging for item-level failures and existing retry behavior for HTTP 429 responses.

Bulk item errors may report their details directly on the error object or under an optional `caused_by` object. Model both forms so a valid Elasticsearch error response is reported as an item failure instead of being rejected as an undecodable bulk response.

Aggregate item errors by normalized detail before logging. Replace dynamic line-and-column coordinates with a position placeholder, sort the most frequent summaries first, and cap both the number and total rendered length of summaries. Report when additional summaries are omitted.

## Risks / Trade-offs

- [Bundle names are not globally unique] → Document that one bundle is intended to ingest into one index. A deployment that combines bundles with the same identifier and relative paths can collide by design.
- [The fallback identifier depends on the working-path layout] → Keep the fallback rule explicit and test it. Users who need a different namespace can place each bundle in its own directory or tracked repository.
- [Truncating SHA-256 introduces a probabilistic collision risk] → Retain 128 bits, which keeps the birthday-collision probability negligible at the expected index sizes, and cover the exact compact encoding with unit tests.
- [Elasticsearch may reject combinations such as `doc_as_upsert` with an ingest pipeline] → Preserve the returned bulk error and document that Elasticsearch remains authoritative for action compatibility.
- [Input failures can occur after earlier batches have been sent] → Preserve the existing streaming and bounded-batch behavior; add request-shape and response tests rather than promising transactional ingestion.
- [Single-source IDs are not idempotent by default] → Preserve the easy default, and allow users who need stable identity to pass `--generate-id=true` explicitly.
- [Split array positions change when records are inserted or reordered] → Use object keys when available, array positions when they are not, and allow explicit `_id` values for stronger identity.
- [Multi-source split changes the input pipeline] → Resolve and validate the file set before opening per-file split readers, while preserving bounded streaming for each reader.

## Migration Plan

This is a CLI behavior change with no index migration. Existing invocations without `--action` switch from `create` to `index`, so users that require create-only behavior must pass `--action=create`. Existing `--action=update` inputs continue to require string IDs. Release documentation should call out the new default, the cardinality-sensitive generated-ID behavior, explicit `--generate-id=true`, and the generated-ID opt-out.

No rollback data migration is required. Reverting the binary restores the prior action default and bulk serialization behavior, but documents already written with generated IDs remain in the target index unless separately removed.
