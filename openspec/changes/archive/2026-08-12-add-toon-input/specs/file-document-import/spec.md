## ADDED Requirements

### Requirement: Toon files stream one document per Toon document
The system SHALL import `.toon` files as structured Toon input where each decoded Toon object emits one JSON object document.

#### Scenario: Toon file is imported
- **WHEN** a file-document input resolves to a `.toon` file
- **THEN** the system parses the file using the Toon input reader
- **AND** each decoded Toon object emits one document

#### Scenario: Toon file is included with multiple file inputs
- **WHEN** file-document input contains a `.toon` file and other supported files
- **THEN** the `.toon` file participates in the existing deterministic file input order
- **AND** documents decoded from that `.toon` file are emitted at that file's position in the ordered input sequence

#### Scenario: Toon file contains a non-object document
- **WHEN** a `.toon` file contains a document that decodes to a non-object value
- **THEN** importing that file fails
- **AND** the invalid Toon document is not sent to any output
