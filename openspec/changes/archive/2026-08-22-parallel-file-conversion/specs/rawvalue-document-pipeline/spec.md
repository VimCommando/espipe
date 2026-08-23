## MODIFIED Requirements

### Requirement: Elasticsearch bulk output preserves raw document buffering

The system SHALL buffer raw documents for Elasticsearch output and emit valid `_bulk` request bodies without requiring `Value` in the steady-state queue. Unless the user supplies `--batch-size`, the system SHALL use a default batch size of 500 documents for multi-source local input and 5,000 documents for single-file streaming input and other input modes.

#### Scenario: Multi-source local input uses smaller default batches

- **WHEN** local input resolves to more than one source file
- **AND** the user does not pass `--batch-size`
- **THEN** Elasticsearch bulk output targets 500 documents per request

#### Scenario: Single-file streaming input retains large default batches

- **WHEN** the user imports one streaming NDJSON, CSV, JSON, or Toon source
- **AND** the user does not pass `--batch-size`
- **THEN** Elasticsearch bulk output targets 5,000 documents per request

#### Scenario: Explicit batch size overrides the source-aware default

- **WHEN** the user passes `--batch-size 750`
- **THEN** Elasticsearch bulk output targets 750 documents per request regardless of input source count

#### Scenario: Bulk queue flushes to Elasticsearch

- **WHEN** the Elasticsearch output flushes one or more buffered documents
- **THEN** it constructs a valid `_bulk` request body for `create` operations using the buffered raw JSON documents
- **AND** Elasticsearch accepts the request without document-shape regressions

#### Scenario: Buffered documents are large

- **WHEN** the queue contains many large documents
- **THEN** the queue retains raw JSON payloads instead of cloned `Value` trees
- **AND** the implementation avoids additional whole-document copies beyond what is required to build the outbound request

