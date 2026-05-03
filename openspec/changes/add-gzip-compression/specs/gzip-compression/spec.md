## ADDED Requirements

### Requirement: Gzip-compressed CSV input
The system SHALL accept local input files ending in `.csv.gz` and parse their decompressed contents using the same CSV-to-JSON behavior as uncompressed `.csv` input.

#### Scenario: Compressed CSV input is imported
- **WHEN** the user runs `espipe` with a local gzip-compressed input file named `events.csv.gz`
- **AND** the decompressed content is valid CSV with a header row
- **THEN** the system emits one JSON document per CSV data row
- **AND** each emitted document uses the same field mapping as an equivalent uncompressed `.csv` input

### Requirement: Gzip-compressed NDJSON input
The system SHALL accept local input files ending in `.ndjson.gz` and parse their decompressed contents using the same line-oriented JSON behavior as uncompressed `.ndjson` input.

#### Scenario: Compressed NDJSON input is imported
- **WHEN** the user runs `espipe` with a local gzip-compressed input file named `events.ndjson.gz`
- **AND** the decompressed content contains valid NDJSON object lines
- **THEN** the system emits one document per decompressed NDJSON line
- **AND** each emitted document preserves the raw JSON object bytes as an equivalent uncompressed `.ndjson` input would

#### Scenario: Invalid compressed NDJSON is rejected
- **WHEN** the user runs `espipe` with a local input file named `events.ndjson.gz`
- **AND** the gzip stream decompresses successfully but a decompressed record is not a JSON object
- **THEN** ingestion fails before that record reaches any output
- **AND** the error follows the existing invalid NDJSON record behavior

#### Scenario: Compressed NDJSON fixture is ingested into localhost Elasticsearch
- **WHEN** the test suite runs the ignored localhost Elasticsearch integration test for gzip-compressed NDJSON input
- **AND** the repository contains a checked-in `.ndjson.gz` fixture with exactly 1,000 valid JSON object records
- **THEN** `espipe` sends the fixture to the localhost Elasticsearch target
- **AND** the target index contains exactly 1,000 documents after refresh

### Requirement: Gzip-compressed NDJSON file output
The system SHALL write gzip-compressed NDJSON when the selected local file output path ends in `.ndjson.gz`.

#### Scenario: Compressed NDJSON output is written
- **WHEN** the user runs `espipe` with a local file output path named `out.ndjson.gz`
- **THEN** the system writes a valid gzip stream to `out.ndjson.gz`
- **AND** decompressing the file yields one NDJSON object line for each document sent to the file output

#### Scenario: Multiple local file inputs can write compressed NDJSON output
- **WHEN** the user runs `espipe` with multiple local file-document inputs
- **AND** the selected local file output path ends in `.ndjson.gz`
- **THEN** startup accepts the output path as an NDJSON file output
- **AND** the system writes the imported documents as compressed NDJSON

### Requirement: Unsupported gzip file formats are rejected
The system SHALL reject gzip-compressed local file paths that do not use the supported `.csv.gz` or `.ndjson.gz` input suffixes or `.ndjson.gz` output suffix.

#### Scenario: Unsupported compressed input suffix is rejected
- **WHEN** the user runs `espipe` with a local input file named `events.json.gz`
- **THEN** startup fails before sending any output
- **AND** the error identifies the file extension or compressed format as unsupported

#### Scenario: Unsupported compressed output suffix is rejected
- **WHEN** the user runs `espipe` with a local file output path named `out.csv.gz`
- **THEN** startup fails before writing compressed output
- **AND** the error identifies the file extension or compressed output format as unsupported

### Requirement: Uncompressed file behavior is preserved
The system SHALL preserve existing behavior for uncompressed `.csv`, `.ndjson`, stdin, stdout, and Elasticsearch outputs.

#### Scenario: Uncompressed CSV and NDJSON behavior is unchanged
- **WHEN** the user runs `espipe` with uncompressed `.csv` or `.ndjson` local input
- **THEN** the system parses and emits documents using the existing uncompressed behavior

#### Scenario: Stdout and Elasticsearch outputs are unchanged
- **WHEN** the user runs `espipe` with stdout or Elasticsearch output
- **THEN** the selected output behaves as it did before gzip file compression support
