## MODIFIED Requirements

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

#### Scenario: Remote HTTPS Toon input is provided
- **WHEN** the user runs `espipe` with an input URI ending in `.toon` over `https://`
- **THEN** the program performs an unauthenticated HTTPS GET for that resource
- **AND** it makes the response body available to the Toon input pipeline before the ingest loop starts

### Requirement: Remote input format is determined by URL extension or HTTP metadata
The system SHALL determine supported remote input formats from the URL path extension when present, and otherwise fall back to HTTP response metadata from a request that advertises CSV, NDJSON-oriented JSON, and Toon support.

#### Scenario: Supported remote extension is used
- **WHEN** the remote URL path ends in `.csv`, `.ndjson`, `.json`, or `.toon`
- **THEN** the input is accepted for remote fetching

#### Scenario: URL has no supported extension but content type is recognized
- **WHEN** the remote URL path does not end in `.csv`, `.ndjson`, `.json`, or `.toon`
- **AND** the HTTPS response `Content-Type` maps to supported CSV, NDJSON, JSON, or Toon input
- **THEN** the input is accepted for remote fetching

#### Scenario: URL and response metadata are both unrecognized
- **WHEN** the remote URL path does not end in a supported extension
- **AND** the HTTPS response `Content-Type` does not map to supported CSV, NDJSON, JSON, or Toon input
- **THEN** startup fails with an explicit unsupported-input error
