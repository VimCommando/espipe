## ADDED Requirements

### Requirement: Multi-source file output order is unspecified

The system SHALL combine and de-duplicate concrete file inputs and glob matches deterministically before scheduling conversion. It SHALL emit each source result as conversion completes and SHALL NOT delay completed sources solely to restore lexicographic output order.

#### Scenario: Later file completes first

- **WHEN** a later source path finishes conversion before an earlier source path
- **THEN** the later source document may reach the output first

#### Scenario: Same file appears more than once

- **WHEN** a file is provided directly and also matched by a glob pattern
- **THEN** the system processes the resolved source once
- **AND** emits each document from that source at most once

### Requirement: Local import summaries distinguish documents from files

The system SHALL report piped documents, evaluated documents, and discovered source files as `Piped X of Y docs from Z files ...`. A skipped source SHALL count toward the discovered file count but SHALL NOT add a document to the evaluated count.

#### Scenario: Some discovered files are skipped

- **WHEN** a local import discovers 10 source files
- **AND** 3 files are skipped without producing documents
- **AND** the remaining files produce 7 documents that are sent successfully
- **THEN** the completion summary begins `Piped 7 of 7 docs from 10 files`

## MODIFIED Requirements

### Requirement: Toon files stream one document per Toon document

The system SHALL import `.toon` files as structured Toon input where each decoded Toon object emits one JSON object document. Documents within one Toon source SHALL preserve their source order. A Toon source within a multi-source file import MAY be emitted before or after other sources according to file conversion completion order.

#### Scenario: Toon file is imported

- **WHEN** a file-document input resolves to a `.toon` file
- **THEN** the system parses the file using the Toon input reader
- **AND** each decoded Toon object emits one document

#### Scenario: Toon file is included with multiple file inputs

- **WHEN** file-document input contains a `.toon` file and other supported files
- **THEN** documents within the Toon file retain their source order
- **AND** the Toon file may be emitted before or after other sources according to conversion completion order

#### Scenario: Toon file contains a non-object document

- **WHEN** a `.toon` file contains a document that decodes to a non-object value
- **THEN** importing that file fails
- **AND** the invalid Toon document is not sent to any output

## REMOVED Requirements

### Requirement: File document import order is deterministic

**Reason**: Restoring lexicographic output order adds head-of-line blocking and retained conversion results without affecting Elasticsearch identity or correctness.

**Migration**: Consumers that require ordering must sort emitted documents by `origin.path` and `origin.filename`.
