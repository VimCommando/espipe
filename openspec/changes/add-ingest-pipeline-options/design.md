## Context

`espipe` sends documents to Elasticsearch through bounded bulk workers. A separate in-progress template change adds a preflight path for installing composable index templates before any `_bulk` request is sent. Ingest pipeline support should reuse that preflight shape while adding one extra output concern: bulk requests may need to target the installed pipeline directly.

The Elasticsearch ingest put pipeline API accepts a JSON pipeline definition at `PUT /_ingest/pipeline/{pipeline_name}`. Index templates can also reference ingest pipelines through settings such as `index.default_pipeline`, which means template and pipeline options must be checked together before document ingestion starts.

## Goals / Non-Goals

**Goals:**

- Add `--pipeline <path>` for Elasticsearch outputs.
- Add `--pipeline-name <name>` as an optional name override.
- Read and validate `.json` pipeline files before any documents are sent.
- Install the pipeline with the Elasticsearch ingest put pipeline API before the first bulk request.
- When `--pipeline` is provided without `--template`, include the selected pipeline as the `_bulk` pipeline target.
- Support `_none` as a special bulk pipeline target that disables an index default pipeline for the request while acknowledging that Elasticsearch final pipelines still run.
- When `--pipeline` and `--template` are both provided, fail before ingestion if the template does not refer to the selected pipeline.
- When a template refers to a pipeline and no `--pipeline` is provided, verify the pipeline exists on the cluster before ingestion.
- Keep runs without `--pipeline` and without template pipeline references unchanged.

**Non-Goals:**

- Supporting `.jsonc` or `.json5` pipeline files.
- Supporting ingest pipeline simulation.
- Supporting pipeline deletion, rollback, or overwrite controls.
- Inferring or mutating template settings to add a pipeline reference.
- Applying pipeline options to file or stdout outputs.

## Decisions

Pass optional pipeline configuration from `main.rs` into the Elasticsearch output setup alongside template configuration: pipeline path and optional pipeline name override. `--pipeline-name` is only meaningful with `--pipeline`; reject it when `--pipeline` is absent except for the reserved `_none` handling described below. Reject pipeline-related options for non-Elasticsearch outputs before opening or reading input content, matching template option behavior.

Read the pipeline file during Elasticsearch preflight. Only `.json` is supported for this change. Parse with strict `serde_json` semantics and serialize the parsed value as normalized JSON for the request body. File read, parse, and empty-name errors should identify the pipeline path or name and be written to stderr.

Derive the default `{pipeline_name}` from the pipeline file name without its final extension. For example, `--pipeline ./pipelines/geoip.json` installs pipeline `geoip`. If `--pipeline-name <name>` is provided, use that exact name instead. Reject an empty derived or explicit pipeline name.

Install the pipeline before any `_bulk` request by sending `PUT /_ingest/pipeline/{pipeline_name}` with `Content-Type: application/json`. Treat any non-2xx response, auth failure, TLS failure, timeout, or transport failure as fatal. This mirrors template preflight ordering so no document batch can be sent after a failed pipeline setup.

When `--pipeline` is provided without `--template`, append the selected pipeline to each bulk request as the request-level `pipeline` query parameter. Keep the NDJSON action metadata unchanged, so the selected pipeline applies consistently to all documents in the request. This uses Elasticsearch's bulk API pipeline targeting instead of mutating every action line.

Treat `_none` as a reserved pipeline target value for the bulk API, not as an ingest pipeline resource name to install or verify. Users select it with `--pipeline-name _none` and no `--pipeline` file. In that mode, `espipe` does not send `PUT /_ingest/pipeline/_none`; it sends `_bulk` requests with `pipeline=_none`, which disables the index default ingest pipeline for those requests. This does not disable an index final pipeline because Elasticsearch always runs final pipelines when configured.

When both `--pipeline` and `--template` are provided, parse the template JSON and extract ingest pipeline references from supported index settings. The initial check should include `template.settings.index.default_pipeline` and equivalent flattened `template.settings["index.default_pipeline"]`. If the template does not refer to the selected pipeline name, fail before installing cluster state or sending documents. `espipe` must not silently install a pipeline that the template will not use.

When a template defines a default pipeline and `--pipeline` is not provided, verify that the referenced pipeline exists by calling `GET /_ingest/pipeline/{pipeline_name}` before sending documents. A missing pipeline or failed existence check is fatal because the target index would be created with a template that refers to an unavailable pipeline.

When a template defines no pipeline and `--pipeline` is not provided, preserve existing template behavior. When both options are omitted, preserve current Elasticsearch bulk behavior.

## Risks / Trade-offs

- Template settings can be represented as nested objects or flattened keys -> support the common nested and flattened `index.default_pipeline` forms and treat unsupported shapes as no detected pipeline reference.
- A template may reference `index.final_pipeline` as well as `index.default_pipeline` -> `_none` only disables the default pipeline for a bulk request; final pipelines still run by Elasticsearch design.
- Installing a pipeline and then failing the template/pipeline consistency check could leave cluster state behind -> run local consistency checks before sending preflight write requests where both files are available.
- Request-level bulk pipeline targeting does not apply when a template is expected to set the default pipeline -> only add the `_bulk` pipeline query parameter when no template is provided, avoiding hidden behavior differences from template-created indices.
- Pipeline existence can change between preflight and index creation -> preflight catches common failures, while Elasticsearch remains the final authority during indexing.

## Migration Plan

No migration is required. Existing invocations without `--pipeline` and without template pipeline references behave as they do today. Rollback removes the CLI options, pipeline config plumbing, ingest pipeline preflight request, template/pipeline checks, and bulk request pipeline query parameter.
