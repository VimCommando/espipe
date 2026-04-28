## 1. CLI and Configuration

- [ ] 1.1 Add a `--pipeline <path>` CLI argument in `src/main.rs`.
- [ ] 1.2 Add a `--pipeline-name <name>` CLI argument that requires `--pipeline` when provided.
- [ ] 1.3 Allow `--pipeline-name _none` without `--pipeline` as a reserved bulk request pipeline target.
- [ ] 1.4 Add pipeline configuration plumbing from `main.rs` into `Output::try_new` and Elasticsearch output setup.
- [ ] 1.5 Reject pipeline-related options for file and stdout outputs with a clear stderr error before opening or reading input content.
- [ ] 1.6 Preserve existing behavior when `--pipeline` is omitted and no template-defined pipeline must be checked.

## 2. Pipeline File Handling

- [ ] 2.1 Read the pipeline file before document ingestion starts.
- [ ] 2.2 Parse `.json` pipeline files as strict JSON and fail with a path-specific error on invalid JSON.
- [ ] 2.3 Derive the default pipeline name from the pipeline file name without its final extension.
- [ ] 2.4 Apply `--pipeline-name` as an explicit pipeline name override and reject empty names.
- [ ] 2.5 Send parsed pipeline definitions as normalized JSON request bodies.

## 3. Elasticsearch Pipeline Preflight

- [ ] 3.1 Send `PUT /_ingest/pipeline/{pipeline_name}` with `Content-Type: application/json`.
- [ ] 3.2 Ensure the pipeline request completes successfully before any `_bulk` request can be sent.
- [ ] 3.3 Treat non-2xx pipeline responses as fatal and include status plus available error details on stderr.
- [ ] 3.4 Treat pipeline transport/auth/TLS failures as fatal before bulk ingestion.

## 4. Bulk Pipeline Targeting

- [ ] 4.1 Add optional bulk request pipeline targeting to Elasticsearch bulk output.
- [ ] 4.2 Include the selected pipeline as the `_bulk` request-level `pipeline` query parameter when `--pipeline` is provided without `--template`.
- [ ] 4.3 Ensure `--pipeline-name` overrides the derived name for bulk request targeting.
- [ ] 4.4 Ensure bulk action metadata remains unchanged when request-level pipeline targeting is used.
- [ ] 4.5 Ensure no request-level `pipeline` query parameter is added when `--template` is provided.
- [ ] 4.6 Ensure `--pipeline-name _none` without `--pipeline` sends `_bulk` requests with `pipeline=_none`.
- [ ] 4.7 Ensure `_none` does not trigger a `PUT /_ingest/pipeline/_none` request or pipeline existence check.

## 5. Template and Pipeline Integration

- [ ] 5.1 Extract `index.default_pipeline` from supported nested template settings.
- [ ] 5.2 Extract `index.default_pipeline` from supported flattened template settings.
- [ ] 5.3 When `--template` and `--pipeline` are both provided, abort before document ingestion if the template does not refer to the selected pipeline.
- [ ] 5.4 When `--template` and `--pipeline` are both provided, run consistency checks before sending preflight write requests.
- [ ] 5.5 When `--template` defines a default pipeline and `--pipeline` is omitted, send `GET /_ingest/pipeline/{pipeline_name}` to verify the pipeline exists.
- [ ] 5.6 Abort before document ingestion when a template-defined pipeline is missing or cannot be verified.
- [ ] 5.7 Preserve existing template preflight behavior when the template defines no pipeline.

## 6. Tests

- [ ] 6.1 Add CLI tests for `--pipeline`, `--pipeline-name`, `--pipeline-name` without `--pipeline`, reserved `_none`, empty names, and non-Elasticsearch outputs.
- [ ] 6.2 Add tests for unreadable and invalid JSON pipeline files.
- [ ] 6.3 Add Elasticsearch output tests proving pipeline request path, method, content type, normalized JSON body, and request ordering before `_bulk`.
- [ ] 6.4 Add tests proving rejected pipeline requests and transport failures abort before any `_bulk` request.
- [ ] 6.5 Add tests proving `_bulk` requests include the `pipeline` query parameter only when `--pipeline` is provided without `--template`.
- [ ] 6.6 Add tests proving `_none` sends `pipeline=_none`, skips pipeline installation, and documents final-pipeline behavior expectations.
- [ ] 6.7 Add tests for template/pipeline agreement, missing template pipeline references, and mismatched template pipeline names.
- [ ] 6.8 Add tests for template-defined pipeline existence checks when `--pipeline` is omitted.
- [ ] 6.9 Add tests proving runs without pipeline options and without template pipeline references remain unchanged.
