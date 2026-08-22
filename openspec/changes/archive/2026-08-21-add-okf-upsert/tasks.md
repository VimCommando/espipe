## 1. CLI and identity configuration

- [x] 1.1 Extend the bulk action enum and CLI help with `upsert`, and change the default action to `index`.
- [x] 1.2 Add `--generate-id=true|false` while preserving an omitted `auto` mode that defaults to enabled for multi-source and disabled for single-source inputs.
- [x] 1.3 Add the stable digest dependency and implement canonical bundle/path ID construction using the agreed bundle identifier and relative path rules.

## 2. File identity and document metadata

- [x] 2.1 Introduce an internal document envelope that preserves the owned raw JSON body while carrying optional transport identity metadata.
- [x] 2.2 Resolve the bundle identifier from the tracked Git root directory name, with the agreed parent-directory fallback for untracked working paths.
- [x] 2.3 Normalize local source paths relative to the working directory, reject outside paths for multi-source inputs including symlink escapes, and produce `origin.scheme`, relative `origin.path`, and `origin.filename` for every local file document.
- [x] 2.4 Generate file IDs only when enabled and no explicit top-level `_id` exists; preserve explicit string IDs and reject non-string IDs.
- [x] 2.5 Add unit coverage for root-level and nested origins, stable IDs across changed contents, repeated checkout paths, explicit-ID precedence, and `--generate-id=false`.

## 3. Elasticsearch bulk serialization

- [x] 3.1 Serialize `create` and `index` metadata with an available ID, omit `_id` when no ID exists, and keep source bodies free of transport-only `_id` fields.
- [x] 3.2 Serialize `update` payloads with `doc` and `upsert` payloads with `doc` plus `doc_as_upsert: true`.
- [x] 3.3 Enforce required IDs for `update` and `upsert` while preserving automatic Elasticsearch IDs for ID-less `create` and `index` operations.
- [x] 3.4 Extend bulk response parsing for update items, including successful `noop` results, and preserve existing error counting and retry behavior.
- [x] 3.5 Add unit tests covering exact bulk NDJSON for every action, explicit and generated IDs, missing IDs, non-string IDs, and update/no-op response handling.

## 4. End-to-end behavior and documentation

- [x] 4.1 Add integration coverage for ingesting an OKF-style Markdown glob, re-ingesting unchanged and changed files, and ingesting a newly added file without creating duplicate IDs.
- [x] 4.2 Add coverage proving `--generate-id=false` delegates ID assignment to Elasticsearch for `create` and `index`, while `update` and `upsert` reject missing IDs.
- [x] 4.3 Update the local-source specification and README action documentation to describe source cardinality, per-file split behavior, `origin`, the new action set, the `index` default, and explicit generated-ID controls.
- [x] 4.4 Run formatting, unit/integration tests, lint checks, and strict OpenSpec validation.

## 5. Source cardinality and per-file split behavior

- [x] 5.1 Resolve explicit paths and globs into a single-source or multi-source input before selecting parser behavior.
- [x] 5.2 Apply `--split` independently to every file in a multi-source input while preserving per-file origin and identity state.
- [x] 5.3 Carry split object keys and array indexes alongside raw documents so they can serve as typed ID discriminators without depending on document content.
- [x] 5.4 Use stable typed discriminators for split documents and ordinary multi-document streams, including explicit single-source opt-in.
- [x] 5.5 Add coverage for omitted, explicit-true, and explicit-false ID modes across single-source and multi-source inputs.
- [x] 5.6 Add coverage for multi-source path rejection, symlink escapes, per-file split origins, split key/index IDs, and external single-source inputs.
- [x] 5.7 Re-run implementation verification and strict OpenSpec validation after the revised behavior is implemented.

## 6. Compact generated ID representation

- [x] 6.1 Replace full SHA-256 hexadecimal generated IDs with the first 128 bits encoded as URL-safe Base64 without padding, update the affected specs and documentation, add exact-format coverage, and rerun verification.

## 7. Multi-source discovery policies

- [x] 7.1 Add `--symlinks=skip|fail|follow` and `--hidden=skip|fail|include` with skip defaults, filter discovery candidates before source-cardinality classification, preserve lexical symlink paths when following, add coverage and documentation, and rerun verification.

## 8. Markdown frontmatter tolerance

- [x] 8.1 Allow duplicate Markdown frontmatter mapping keys with a warning, use the last value, preserve fatal errors for other invalid frontmatter, add regression coverage and documentation, and rerun verification.

## 9. Bulk error response compatibility

- [x] 9.1 Accept Elasticsearch bulk item errors with either nested or top-level error details, add regression coverage for errors without `caused_by`, and rerun verification.

## 10. Bounded bulk error summaries

- [x] 10.1 Normalize dynamic error positions, aggregate repeated bulk failures, bound the rendered warning, add regression coverage, and rerun verification.
