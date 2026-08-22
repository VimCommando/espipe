## Why

Git-hosted OKF bundles identify documents by their file paths, but the current `index` and `update` behavior cannot safely re-ingest those documents without either creating duplicate Elasticsearch documents or applying the wrong update semantics. The bulk action must be explicit, and file-derived IDs must remain stable when document content changes or the bundle is checked out again.

## What Changes

- Add explicit `create`, `index`, `update`, and `upsert` Elasticsearch bulk actions.
- Change the default bulk action from `create` to `index`.
- Generate deterministic IDs for multi-source local file inputs by default from the bundle identifier, working-directory-relative source path, and document discriminator.
- Keep single-source inputs easy to use by not generating IDs unless `--generate-id=true` is explicitly provided.
- Add explicit `--generate-id=true|false` controls; an omitted value uses the source-cardinality default.
- Apply `--split` independently to each file in a multi-source input.
- Reject multi-source files outside the working directory.
- Add configurable multi-source discovery policies for symlinks and hidden path components.
- Warn and continue when Markdown frontmatter repeats a mapping key, using the last value.
- Give explicit top-level string `_id` values precedence over generated IDs.
- Encode generated IDs as compact 128-bit URL-safe Base64 values rather than full SHA-256 hexadecimal strings.
- Emit the appropriate Elasticsearch bulk metadata and update payload, including `doc_as_upsert: true` for `upsert`.
- Handle update and no-op bulk responses without treating Elasticsearch no-ops as failures.
- Preserve bulk item error reporting when Elasticsearch omits an optional nested `caused_by` detail.
- Bound and normalize bulk error summaries so repeated position-specific failures do not flood the warning log.
- Replace file-specific `file.path` and `file.name` metadata requirements with generalized `origin` metadata. Local file origins use a relative path and filename.
- Preserve the existing non-goal that source deletions are not synchronized to Elasticsearch.

## Capabilities

### New Capabilities

- `elasticsearch-bulk-actions`: Defines supported bulk actions, deterministic ID selection, generated-ID controls, bulk request payloads, and response handling.

### Modified Capabilities

- `file-document-import`: Replace file-specific metadata requirements with relative local-file `origin` metadata and expose source identity for deterministic IDs.

## Impact

- Affected CLI: `--action` values and default, plus the `--generate-id`, `--symlinks`, and `--hidden` options.
- Affected input pipeline: source-cardinality resolution, per-file split processing, relative source identity, origin metadata, explicit `_id` precedence, and Markdown frontmatter parsing.
- Affected Elasticsearch output: bulk metadata generation, update/upsert payloads, and bulk response parsing.
- Affected documentation and tests: action semantics, OKF re-ingestion examples, deterministic IDs, repeated ingestion, added files, and no-op responses.
- No new external service is required. The implementation may need a stable hashing dependency if the selected ID representation uses a digest.
