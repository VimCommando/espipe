## Purpose

Define how `espipe` retrieves unauthenticated remote inputs over HTTPS and feeds them into the existing CSV and NDJSON ingest pipeline.

## Requirements

### Requirement: HTTPS remote inputs are fetchable before ingestion
The system SHALL accept supported `https://` input URIs and fetch the remote body before document ingestion begins.

#### Scenario: Remote HTTPS NDJSON input is provided
- **WHEN** the user runs `espipe` with an input URI ending in `.ndjson` over `https://`
- **THEN** the program performs an unauthenticated HTTPS GET for that resource
- **AND** it makes the response body available to the existing JSON input pipeline before the ingest loop starts

#### Scenario: Remote HTTPS CSV input is provided
- **WHEN** the user runs `espipe` with an input URI ending in `.csv` over `https://`
- **THEN** the program performs an unauthenticated HTTPS GET for that resource
- **AND** it makes the response body available to the existing CSV input pipeline before the ingest loop starts

### Requirement: Remote input format is determined by URL extension or HTTP metadata
The system SHALL determine supported remote input formats from the URL path extension when present, and otherwise fall back to HTTP response metadata from a request that advertises CSV and NDJSON-oriented JSON support.

#### Scenario: Supported remote extension is used
- **WHEN** the remote URL path ends in `.csv`, `.ndjson`, or `.json`
- **THEN** the input is accepted for remote fetching

#### Scenario: URL has no supported extension but content type is recognized
- **WHEN** the remote URL path does not end in `.csv`, `.ndjson`, or `.json`
- **AND** the HTTPS response `Content-Type` maps to supported CSV or NDJSON input
- **THEN** the input is accepted for remote fetching

#### Scenario: URL and response metadata are both unrecognized
- **WHEN** the remote URL path does not end in a supported extension
- **AND** the HTTPS response `Content-Type` does not map to supported CSV or NDJSON input
- **THEN** startup fails with an explicit unsupported-input error

### Requirement: Remote JSON inputs preserve line-delimited parsing behavior
The system SHALL treat fetched remote `.ndjson` and `.json` content as line-delimited JSON input.

#### Scenario: Remote `.json` input is consumed
- **WHEN** the user provides a remote URL ending in `.json`
- **THEN** the input is parsed using the same line-oriented JSON reader behavior as `.ndjson`
- **AND** each consumed line must be a valid JSON object before it reaches an output

#### Scenario: Remote `.json` payload is not valid NDJSON
- **WHEN** the fetched `.json` payload does not match the required line-delimited JSON object format
- **THEN** startup fails gracefully
- **AND** the user sees the message `JSON payload does not look like required NDJSON input format.`

### Requirement: Non-success remote fetches fail before ingestion
The system SHALL reject remote inputs that cannot be retrieved successfully.

#### Scenario: Remote server returns an error response
- **WHEN** the HTTPS GET returns a non-success HTTP status
- **THEN** the program exits before document ingestion starts
- **AND** it reports the fetch failure to the user

#### Scenario: Remote request cannot be completed
- **WHEN** the HTTPS GET fails because of a DNS, TLS, timeout, or transport error
- **THEN** the program exits before document ingestion starts
- **AND** it reports the fetch failure to the user

### Requirement: Remote input support is unauthenticated and HTTPS-only
The system SHALL limit this input mode to unauthenticated HTTPS requests.

#### Scenario: HTTP input URI is provided
- **WHEN** the user provides an `http://` input URI
- **THEN** the input is rejected as unsupported for remote fetching

#### Scenario: HTTPS input requires custom authentication
- **WHEN** the remote resource requires credentials not present in the URI
- **THEN** the program does not attempt custom remote-input authentication for this change
