## Purpose

Define how `espipe` imports Toon-formatted inputs as JSON object documents.

## Requirements

### Requirement: Toon inputs stream object documents
The system SHALL accept Toon input and emit one JSON object document for each Toon document decoded from the input stream. Multi-document Toon input SHALL separate documents with a line whose trimmed content is exactly `---`.

#### Scenario: Local Toon file is imported
- **WHEN** the user runs `espipe` with a local input path ending in `.toon`
- **THEN** the system parses the file as Toon input
- **AND** each decoded Toon document emits one JSON object document

#### Scenario: Toon input contains multiple documents
- **WHEN** a `.toon` input contains multiple Toon object documents separated by `---` lines
- **THEN** the system emits one output document for each decoded Toon document
- **AND** document order matches the input order

#### Scenario: Toon input contains one top-level tabular object array
- **WHEN** a `.toon` input decodes to an object with exactly one top-level field whose value is an array of objects
- **THEN** the system emits one JSON object document for each object in that array
- **AND** document order matches the array row order

#### Scenario: Toon input is large
- **WHEN** a `.toon` input contains many documents separated by `---` lines
- **THEN** the system parses the Toon input incrementally from a reader
- **AND** it does not require the entire Toon input to be materialized before emitting the first document

### Requirement: Toon documents must be JSON objects
The system SHALL reject Toon documents that cannot be represented as JSON object documents.

#### Scenario: Toon object document is imported
- **WHEN** a Toon document decodes to a mapping/object value
- **THEN** the emitted document is the corresponding JSON object

#### Scenario: Direct Toon array document is rejected
- **WHEN** a Toon document decodes directly to an array value
- **THEN** importing that input fails
- **AND** no non-object Toon document is sent to any output

#### Scenario: Top-level object array row is not an object
- **WHEN** a Toon document decodes to an object with exactly one top-level array field
- **AND** any array row is not an object
- **THEN** importing that input fails
- **AND** the invalid Toon row is not sent to any output

#### Scenario: Toon scalar document is rejected
- **WHEN** a Toon document decodes to a scalar or null value
- **THEN** importing that input fails
- **AND** the diagnostic identifies the Toon input as having an invalid document shape

### Requirement: Toon parse failures stop ingestion
The system SHALL fail before sending any further output when Toon input cannot be parsed.

#### Scenario: Toon syntax is invalid
- **WHEN** a `.toon` input contains invalid Toon syntax
- **THEN** ingestion fails with a Toon parse error
- **AND** the error identifies the input and document position when available from the parser

#### Scenario: Toon stream fails after earlier documents
- **WHEN** a `.toon` input contains valid documents followed by invalid Toon content
- **THEN** ingestion stops at the parse failure
- **AND** no documents after the invalid content are sent

### Requirement: Toon input uses the Toon parser with an incremental reader
The system SHALL use the `toon-format` parser dependency for Toon document parsing and SHALL own the incremental `---`-separated document reader in `espipe`.

#### Scenario: Dependency is configured
- **WHEN** the project dependencies are resolved
- **THEN** the Toon parser dependency is sourced from crates.io package `toon-format`

#### Scenario: Toon input is read
- **WHEN** the system consumes Toon input
- **THEN** it reads each `---`-separated document chunk incrementally
- **AND** it decodes each chunk with the Toon parser
