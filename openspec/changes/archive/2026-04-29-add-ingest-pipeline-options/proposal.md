## Why

Users often need an ingest pipeline in place before bulk indexing begins, and they may also need each bulk request to target that pipeline explicitly. Supporting pipeline loading alongside index template loading makes one-shot imports reproducible without requiring a separate Elasticsearch setup step.

## What Changes

- Add a `--pipeline <path>` command-line option for Elasticsearch outputs.
- Add a `--pipeline-name <name>` override, defaulting to the pipeline file name without its extension.
- Read a `.json` ingest pipeline definition before sending any documents and install it with the Elasticsearch ingest put pipeline API.
- When `--pipeline` is provided without `--template`, include the selected pipeline as the `_bulk` pipeline target.
- Allow `_none` as a special bulk pipeline target to disable an index default ingest pipeline for the request, while preserving Elasticsearch final pipeline behavior.
- When `--pipeline` and `--template` are both provided, abort if the template does not define the selected pipeline.
- When a template defines a default pipeline but `--pipeline` is not provided, verify that the referenced pipeline exists on the cluster and abort if it does not.
- Keep runs without pipeline or template configuration unchanged.

## Capabilities

### New Capabilities

- `elasticsearch-ingest-pipeline`: Defines CLI-driven Elasticsearch ingest pipeline preflight behavior and bulk pipeline targeting.

### Modified Capabilities

None.

## Impact

- Affected CLI surface: adds `--pipeline <path>` and `--pipeline-name <name>`.
- Affected Elasticsearch preflight: adds ingest pipeline parsing, naming, installation, and pipeline existence checks through Elasticsearch ingest APIs.
- Affected bulk output: bulk requests may include a `pipeline` target when `--pipeline` is provided without a template or when `_none` is requested to disable default pipeline execution.
- Affected template behavior: template definitions that reference ingest pipelines must be checked for consistency with provided pipeline options or cluster state.
- Tests should cover pipeline name derivation and override, invalid pipeline files, ingest put pipeline request behavior, bulk request pipeline targeting, template/pipeline mismatch aborts, and template-defined pipeline existence checks.
