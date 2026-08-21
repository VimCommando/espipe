## Context

The current CLI exposes `create`, `index`, and `update`, defaults to `create`, and constructs bulk bodies directly from `Box<RawValue>` documents. `update` extracts a top-level string `_id`, but the bulk response model only recognizes `create` and `index` items. Local file imports already carry source metadata internally as `origin`, although the metadata currently contains checkout-dependent paths and is not emitted consistently for direct file inputs.

The proposal and delta specs define the required user-visible behavior. This design describes how to add it without making non-Elasticsearch outputs emit bulk metadata.

## Goals / Non-Goals

**Goals:**

- Keep Elasticsearch action selection at the bulk NDJSON boundary.
- Make generated file IDs independent of content, timestamps, and absolute checkout paths.
- Preserve raw JSON document bodies for file and stdout outputs.
- Keep document identity metadata separate from the source body until Elasticsearch bulk serialization.
- Extend bulk response handling to recognize update results and no-ops.

**Non-Goals:**

- Synchronize files deleted from the bundle into Elasticsearch.
- Discover or clone remote Git repositories.
- Add a bundle manifest or a user-supplied bundle-ID option.
- Change the semantics Elasticsearch applies after it receives an `update` or `upsert` payload.
- Add generated IDs to non-file input forms.

## Decisions

### Use explicit bulk action mappings

Extend the existing `BulkAction` value enum with `Upsert` and change its default to `Index`. Keep action-specific serialization in the Elasticsearch output layer:

- `create` emits `create` metadata and a source line.
- `index` emits `index` metadata and a source line.
- `update` emits `update` metadata and a payload containing `doc`.
- `upsert` emits `update` metadata and a payload containing `doc` plus `doc_as_upsert: true`.

This keeps Elasticsearch responsible for action semantics. The client only selects the bulk operation and constructs the documented payload. File and stdout outputs continue to write raw documents and do not serialize action metadata.

### Carry generated identity alongside the raw body

Introduce an internal document envelope containing the existing owned `Box<RawValue>` plus optional transport identity metadata. The input layer sets the generated identity for local file documents when enabled. The Elasticsearch output resolves explicit top-level `_id` values, gives them precedence, and removes the transport-only field while building the bulk body. File and stdout outputs ignore the transport identity and continue writing the raw document body.

This avoids injecting generated IDs into user-visible raw output and avoids reparsing documents solely to discover generated file identity. Existing raw JSON buffering remains intact.

### Derive IDs from a canonical bundle/path key

For a local file document without an explicit `_id`, construct a canonical UTF-8 key from:

```text
<bundle identifier> + "\0" + <working-directory-relative path>
```

Normalize the relative path to use `/` separators, remove a leading `./`, and retain the filename extension. Hash the key with SHA-256 and encode the digest as lowercase hexadecimal. The resulting 64-character value is the generated Elasticsearch ID.

Resolve the bundle identifier by locating the tracked Git working-tree root and using its directory name. If no tracked Git working tree is found, use the agreed parent-directory fallback for the working path. Do not include the absolute checkout path in the canonical key.

The same path normalization feeds local `origin.path` and `origin.filename` metadata. File discovery must reject or normalize paths so a generated ID never contains an absolute checkout prefix.

### Generate IDs in file input mode only

Add a top-level `--generate-id` boolean option defaulting to true. Pass the setting into file-document input construction. File documents receive generated identity only when the option is enabled and no explicit top-level `_id` exists. CSV, NDJSON, JSON, Toon, stdin, split, and remote inputs do not receive generated identity from this feature.

When `update` or `upsert` requires an ID and neither an explicit nor generated ID exists, fail while constructing the operation. `create` and `index` omit `_id` in that case and preserve Elasticsearch's automatic ID behavior.

### Generalize local origin metadata

Build local file origins from the working-directory-relative path for every emitted file document, including a single direct file. Use `scheme: file`, the relative containing directory as `path`, and the final component as `filename`; represent a root-level file's directory as `./`.

Update the existing file-document requirement delta from `file.path` and `file.name` to `origin`. Keep origin metadata in the document body because it is source metadata, while keep generated identity in the internal envelope because it is transport metadata.

### Extend bulk response parsing

Add an `update` variant to the bulk response item model and treat successful update statuses, including `result: "noop"`, as successful counts. Preserve existing logging for item-level failures and existing retry behavior for HTTP 429 responses.

## Risks / Trade-offs

- [Bundle names are not globally unique] → Document that one bundle is intended to ingest into one index. A deployment that combines bundles with the same identifier and relative paths can collide by design.
- [The fallback identifier depends on the working-path layout] → Keep the fallback rule explicit and test it. Users who need a different namespace can place each bundle in its own directory or tracked repository.
- [Adding SHA-256 introduces a dependency] → Use a small, well-supported digest crate and keep the canonical input and hexadecimal encoding covered by unit tests.
- [Elasticsearch may reject combinations such as `doc_as_upsert` with an ingest pipeline] → Preserve the returned bulk error and document that Elasticsearch remains authoritative for action compatibility.
- [Input failures can occur after earlier batches have been sent] → Preserve the existing streaming and bounded-batch behavior; add request-shape and response tests rather than promising transactional ingestion.

## Migration Plan

This is a CLI behavior change with no index migration. Existing invocations without `--action` switch from `create` to `index`, so users that require create-only behavior must pass `--action=create`. Existing `--action=update` inputs continue to require string IDs. Release documentation should call out the new default and the generated-ID opt-out.

No rollback data migration is required. Reverting the binary restores the prior action default and bulk serialization behavior, but documents already written with generated IDs remain in the target index unless separately removed.
