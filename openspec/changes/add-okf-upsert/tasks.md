## 1. CLI and identity configuration

- [ ] 1.1 Extend the bulk action enum and CLI help with `upsert`, and change the default action to `index`.
- [ ] 1.2 Add the `--generate-id` boolean option with a default of true and pass it into local file-document input construction.
- [ ] 1.3 Add the stable digest dependency and implement canonical bundle/path ID construction using the agreed bundle identifier and relative path rules.

## 2. File identity and document metadata

- [ ] 2.1 Introduce an internal document envelope that preserves the owned raw JSON body while carrying optional transport identity metadata.
- [ ] 2.2 Resolve the bundle identifier from the tracked Git root directory name, with the agreed parent-directory fallback for untracked working paths.
- [ ] 2.3 Normalize local file paths relative to the working directory, reject or handle paths outside that scope according to the design, and produce `origin.scheme`, relative `origin.path`, and `origin.filename` for every local file document.
- [ ] 2.4 Generate file IDs only when enabled and no explicit top-level `_id` exists; preserve explicit string IDs and reject non-string IDs.
- [ ] 2.5 Add unit coverage for root-level and nested origins, stable IDs across changed contents, repeated checkout paths, explicit-ID precedence, and `--generate-id=false`.

## 3. Elasticsearch bulk serialization

- [ ] 3.1 Serialize `create` and `index` metadata with an available ID, omit `_id` when no ID exists, and keep source bodies free of transport-only `_id` fields.
- [ ] 3.2 Serialize `update` payloads with `doc` and `upsert` payloads with `doc` plus `doc_as_upsert: true`.
- [ ] 3.3 Enforce required IDs for `update` and `upsert` while preserving automatic Elasticsearch IDs for ID-less `create` and `index` operations.
- [ ] 3.4 Extend bulk response parsing for update items, including successful `noop` results, and preserve existing error counting and retry behavior.
- [ ] 3.5 Add unit tests covering exact bulk NDJSON for every action, explicit and generated IDs, missing IDs, non-string IDs, and update/no-op response handling.

## 4. End-to-end behavior and documentation

- [ ] 4.1 Add integration coverage for ingesting an OKF-style Markdown glob, re-ingesting unchanged and changed files, and ingesting a newly added file without creating duplicate IDs.
- [ ] 4.2 Add coverage proving `--generate-id=false` delegates ID assignment to Elasticsearch for `create` and `index`, while `update` and `upsert` reject missing IDs.
- [ ] 4.3 Update the file-document specification and README action documentation to describe `origin`, the new action set, the `index` default, generated IDs, and the opt-out flag.
- [ ] 4.4 Run formatting, unit/integration tests, lint checks, and strict OpenSpec validation.
