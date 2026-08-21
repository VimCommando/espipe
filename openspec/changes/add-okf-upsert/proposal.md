## Why

Git-hosted OKF bundles identify documents by their file paths, but the current `index` and `update` behavior cannot safely re-ingest those documents without either creating duplicate Elasticsearch documents or applying the wrong update semantics. The bulk action must be explicit, and file-derived IDs must remain stable when document content changes or the bundle is checked out again.

## What Changes

- Add explicit `create`, `index`, `update`, and `upsert` Elasticsearch bulk actions.
- Change the default bulk action from `create` to `index`.
- Generate deterministic IDs for file-document inputs by default from the bundle identifier and working-directory-relative source path.
- Add `--generate-id=false` to disable generated IDs for file inputs.
- Give explicit top-level string `_id` values precedence over generated IDs.
- Emit the appropriate Elasticsearch bulk metadata and update payload, including `doc_as_upsert: true` for `upsert`.
- Handle update and no-op bulk responses without treating Elasticsearch no-ops as failures.
- Replace file-specific `file.path` and `file.name` metadata requirements with generalized `origin` metadata. Local file origins use a relative path and filename.
- Preserve the existing non-goal that source deletions are not synchronized to Elasticsearch.

## Capabilities

### New Capabilities

- `elasticsearch-bulk-actions`: Defines supported bulk actions, deterministic ID selection, generated-ID controls, bulk request payloads, and response handling.

### Modified Capabilities

- `file-document-import`: Replace file-specific metadata requirements with relative local-file `origin` metadata and expose source identity for deterministic IDs.

## Impact

- Affected CLI: `--action` values and default, plus the new `--generate-id` option.
- Affected input pipeline: local file discovery, relative source identity, origin metadata, and explicit `_id` precedence.
- Affected Elasticsearch output: bulk metadata generation, update/upsert payloads, and bulk response parsing.
- Affected documentation and tests: action semantics, OKF re-ingestion examples, deterministic IDs, repeated ingestion, added files, and no-op responses.
- No new external service is required. The implementation may need a stable hashing dependency if the selected ID representation uses a digest.
