## ADDED Requirements

### Requirement: Pipeline option installs an Elasticsearch ingest pipeline
The system SHALL accept `--pipeline <path>` for Elasticsearch outputs and send the JSON file as an ingest pipeline before sending any bulk document request.

#### Scenario: Pipeline is installed before bulk indexing
- **WHEN** the user runs `espipe` with an Elasticsearch output and `--pipeline pipeline.json`
- **THEN** the system reads `pipeline.json`
- **AND** it sends the file contents to Elasticsearch before the first `_bulk` request
- **AND** it sends document batches only after Elasticsearch accepts the pipeline request

#### Scenario: Default pipeline name is derived from file name
- **WHEN** the user passes `--pipeline ./pipelines/geoip.json`
- **THEN** the system sends the pipeline request to `/_ingest/pipeline/geoip`

#### Scenario: Pipeline name is overridden
- **WHEN** the user passes `--pipeline ./pipelines/geoip.json --pipeline-name custom-pipeline`
- **THEN** the system sends the pipeline request to `/_ingest/pipeline/custom-pipeline`

#### Scenario: Empty pipeline name is rejected
- **WHEN** the derived pipeline name or `--pipeline-name` value is empty
- **THEN** startup fails before any documents are sent
- **AND** the error explains that the pipeline name must be non-empty

### Requirement: Pipeline files must be valid JSON
The system SHALL validate `.json` pipeline files before sending them to Elasticsearch.

#### Scenario: Pipeline file is unreadable
- **WHEN** the user passes `--pipeline` with a path that cannot be read
- **THEN** startup fails before any documents are sent
- **AND** the error identifies the pipeline path
- **AND** the error is written to stderr

#### Scenario: Pipeline file is invalid JSON
- **WHEN** the user passes `--pipeline` with a file that is not valid JSON
- **THEN** startup fails before any documents are sent
- **AND** the error identifies the pipeline path and JSON parse failure
- **AND** the error is written to stderr

### Requirement: Pipeline rejection aborts ingestion
The system SHALL abort the run when Elasticsearch rejects the ingest pipeline request.

#### Scenario: Elasticsearch rejects pipeline
- **WHEN** Elasticsearch responds to the pipeline request with a non-2xx status
- **THEN** the system fails the run
- **AND** no bulk document request is sent
- **AND** the error includes the response status and available Elasticsearch error details
- **AND** the error is written to stderr

#### Scenario: Pipeline request cannot be completed
- **WHEN** the pipeline request fails because of authentication, TLS, DNS, timeout, or transport error
- **THEN** the system fails the run
- **AND** no bulk document request is sent
- **AND** the error is written to stderr

### Requirement: Pipeline option targets bulk requests when no template is provided
The system SHALL include the selected ingest pipeline as the `_bulk` request pipeline target when `--pipeline` is provided without `--template`.

#### Scenario: Bulk request uses provided pipeline
- **WHEN** the user runs `espipe` with an Elasticsearch output and `--pipeline pipeline.json`
- **AND** the user does not provide `--template`
- **THEN** each `_bulk` request includes the selected pipeline as the request-level `pipeline` query parameter
- **AND** the bulk action metadata remains otherwise unchanged

#### Scenario: Pipeline name override is used for bulk target
- **WHEN** the user passes `--pipeline pipeline.json --pipeline-name normalized`
- **AND** the user does not provide `--template`
- **THEN** each `_bulk` request targets pipeline `normalized`

### Requirement: None pipeline target disables index default pipeline
The system SHALL support `_none` as a reserved `_bulk` request pipeline target that disables the index default ingest pipeline for the request.

#### Scenario: None pipeline target is used without pipeline file
- **WHEN** the user passes `--pipeline-name _none` without `--pipeline`
- **AND** the user does not provide `--template`
- **THEN** startup succeeds without reading or installing an ingest pipeline file
- **AND** each `_bulk` request includes `pipeline=_none`

#### Scenario: None pipeline target does not disable final pipeline
- **WHEN** the user passes `--pipeline-name _none` without `--pipeline`
- **AND** the target index has a final pipeline configured
- **THEN** the system sends `_bulk` requests with `pipeline=_none`
- **AND** Elasticsearch remains responsible for running the final pipeline

#### Scenario: None pipeline is not installed
- **WHEN** the user passes `--pipeline-name _none` without `--pipeline`
- **THEN** the system does not send a request to `/_ingest/pipeline/_none`
- **AND** the system does not verify that an ingest pipeline named `_none` exists

### Requirement: Template and pipeline options must agree
The system SHALL reject a run when `--pipeline` and `--template` are both provided but the template does not refer to the selected pipeline.

#### Scenario: Template refers to provided pipeline
- **WHEN** the user passes `--template template.json --pipeline pipeline.json --pipeline-name geoip`
- **AND** the template defines `index.default_pipeline` as `geoip`
- **THEN** startup continues after template and pipeline preflight succeeds
- **AND** bulk requests do not add a request-level `pipeline` query parameter

#### Scenario: Template does not refer to provided pipeline
- **WHEN** the user passes `--template template.json --pipeline pipeline.json --pipeline-name geoip`
- **AND** the template does not define `index.default_pipeline` as `geoip`
- **THEN** startup fails before any documents are sent
- **AND** the error explains that the template does not reference the provided pipeline
- **AND** the error is written to stderr

#### Scenario: Template refers to a different pipeline
- **WHEN** the user passes `--template template.json --pipeline pipeline.json --pipeline-name geoip`
- **AND** the template defines `index.default_pipeline` as `other-pipeline`
- **THEN** startup fails before any documents are sent
- **AND** the error identifies the template pipeline and the provided pipeline

### Requirement: Template-defined pipelines must exist when no pipeline is provided
The system SHALL verify that a template-defined ingest pipeline exists on the cluster when `--template` is provided without `--pipeline`.

#### Scenario: Template-defined pipeline exists
- **WHEN** the user passes `--template template.json` without `--pipeline`
- **AND** the template defines `index.default_pipeline` as `geoip`
- **AND** Elasticsearch confirms `/_ingest/pipeline/geoip` exists
- **THEN** startup continues after template preflight succeeds
- **AND** document bulk indexing starts using the existing output flow

#### Scenario: Template-defined pipeline is missing
- **WHEN** the user passes `--template template.json` without `--pipeline`
- **AND** the template defines `index.default_pipeline` as `geoip`
- **AND** Elasticsearch reports that pipeline `geoip` does not exist
- **THEN** startup fails before any documents are sent
- **AND** the error explains that the template references a missing ingest pipeline
- **AND** the error is written to stderr

#### Scenario: Template has no pipeline reference
- **WHEN** the user passes `--template template.json` without `--pipeline`
- **AND** the template does not define an ingest pipeline
- **THEN** no ingest pipeline existence check is required
- **AND** existing template preflight behavior is preserved

### Requirement: Pipeline argument failures occur before input access
The system SHALL validate pipeline-related arguments and output compatibility before opening or reading input content.

#### Scenario: Pipeline option is invalid
- **WHEN** the user provides invalid pipeline-related arguments
- **THEN** startup fails before opening or reading input content
- **AND** the error is written to stderr

#### Scenario: Pipeline option is incompatible with output
- **WHEN** the user provides pipeline-related arguments with a non-Elasticsearch output
- **THEN** startup fails before opening or reading input content
- **AND** the error is written to stderr

### Requirement: Pipeline option only applies to Elasticsearch outputs
The system SHALL reject pipeline-related options when the selected output is not Elasticsearch.

#### Scenario: Pipeline is used with file output
- **WHEN** the user passes `--pipeline pipeline.json` with a file output
- **THEN** startup fails before reading input documents
- **AND** the error explains that `--pipeline` requires an Elasticsearch output
- **AND** the error is written to stderr

#### Scenario: Pipeline is used with stdout output
- **WHEN** the user passes `--pipeline pipeline.json` with stdout output
- **THEN** startup fails before reading input documents
- **AND** the error explains that `--pipeline` requires an Elasticsearch output
- **AND** the error is written to stderr

#### Scenario: Pipeline name is used without pipeline path
- **WHEN** the user passes `--pipeline-name geoip` without `--pipeline`
- **THEN** startup fails before reading input documents
- **AND** the error explains that `--pipeline-name` requires `--pipeline`
- **AND** the error is written to stderr

#### Scenario: Reserved none pipeline name is used without pipeline path
- **WHEN** the user passes `--pipeline-name _none` without `--pipeline`
- **THEN** startup treats `_none` as a reserved bulk pipeline target
- **AND** startup does not fail because `--pipeline` is absent

### Requirement: Runs without pipeline remain unchanged
The system SHALL preserve existing output behavior when `--pipeline` is not provided and no template-defined pipeline must be checked.

#### Scenario: Elasticsearch output without pipeline or template pipeline reference
- **WHEN** the user runs `espipe` with an Elasticsearch output and no `--pipeline`
- **AND** no provided template defines an ingest pipeline
- **THEN** the system does not send an ingest pipeline request
- **AND** document bulk indexing starts using the existing output flow
