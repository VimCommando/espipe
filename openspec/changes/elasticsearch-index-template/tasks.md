## 1. CLI and Configuration

- [x] 1.1 Add a `--template <path>` CLI argument in `src/main.rs`.
- [x] 1.2 Add a `--template-name <name>` CLI argument that requires `--template` when provided.
- [x] 1.3 Add a `--template-overwrite <bool>` CLI argument that defaults to `true` and requires `--template` when explicitly provided.
- [x] 1.4 Add template configuration plumbing from `main.rs` into `Output::try_new`.
- [x] 1.5 Reject template-related options for file and stdout outputs with a clear stderr error before opening or reading input content.
- [x] 1.6 Preserve existing behavior when `--template` is omitted.

## 2. Template File Handling

- [x] 2.1 Read the template file before document ingestion starts.
- [x] 2.2 Parse `.json` template files as strict JSON and fail with a path-specific error on invalid JSON.
- [x] 2.3 Derive the default template name from the template file name without its final extension.
- [x] 2.4 Apply `--template-name` as an explicit template name override and reject empty names.
- [x] 2.5 Inspect `index_patterns` using Elasticsearch multi-target include/exclude syntax and warn when no pattern matches the output target index name or when the check cannot be performed.
- [x] 2.6 Parse `.jsonc` and `.json5` template files with `serde_json5` or equivalent JSON-with-comments/JSON5-compatible syntax.
- [x] 2.7 Send parsed templates as normalized JSON request bodies.

## 3. Elasticsearch Preflight Request

- [x] 3.1 Send `PUT /_index_template/{template_name}` with `Content-Type: application/json` when template overwrite is enabled.
- [x] 3.2 Send `POST /_index_template/{template_name}?create=true` when `--template-overwrite=false` and abort before bulk ingestion if the template already exists.
- [x] 3.3 Ensure no request is sent to the legacy `/_template/{template_name}` API.
- [x] 3.4 Ensure the template request completes successfully before any `_bulk` request can be sent.
- [x] 3.5 Treat non-2xx template responses as fatal and include status plus available error details on stderr.
- [x] 3.6 Treat template transport/auth/TLS failures as fatal before opening or reading input content when feasible, and always before bulk ingestion.

## 4. Tests and Verification

- [x] 4.1 Add tests for unreadable, invalid strict JSON, invalid JSONC, and invalid JSON5 template files.
- [x] 4.2 Add tests for template name derivation from file name, explicit `--template-name`, empty name rejection, `--template-name` without `--template`, and explicit `--template-overwrite` without `--template`.
- [x] 4.3 Add tests for `--template-overwrite=true` and `--template-overwrite=false` request behavior, including existing-template abort before any `_bulk` request.
- [x] 4.4 Add tests that template-related options are rejected for file and stdout outputs before input content is opened or read.
- [x] 4.5 Add Elasticsearch output tests proving template request path, method, content type, normalized JSON body, and composable-only API usage.
- [x] 4.6 Add tests proving index-pattern matching follows Elasticsearch multi-target include/exclude ordering and that mismatches or unverifiable checks emit stderr warnings without aborting.
- [x] 4.7 Add tests proving a rejected template aborts before any `_bulk` request.
- [x] 4.8 Add tests proving no template request is sent when `--template` is omitted.
- [x] 4.9 Add tests proving `.jsonc` block comments and `.json5` syntax are accepted and serialized as valid JSON.
- [x] 4.10 Run `cargo test` and fix any failures.
