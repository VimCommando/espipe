## ADDED Requirements

### Requirement: Toon input produces owned raw JSON documents
The system SHALL convert each decoded Toon object into an owned `Box<serde_json::value::RawValue>` before dispatching it to outputs.

#### Scenario: Toon document is read
- **WHEN** the input reader consumes a valid Toon object document
- **THEN** it returns an owned `Box<RawValue>` for that document
- **AND** it rejects invalid Toon or non-object Toon values before the document reaches any output

#### Scenario: Toon document is sent to an output
- **WHEN** the main ingest loop forwards a parsed Toon document to an output
- **THEN** the sender interface consumes ownership of that `Box<RawValue>`
- **AND** steady-state output buffering does not require retaining a `serde_json::Value` tree for that document
