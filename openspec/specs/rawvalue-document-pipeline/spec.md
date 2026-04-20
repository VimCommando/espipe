## ADDED Requirements

### Requirement: Input pipeline preserves pass-through JSON as raw values
The system SHALL represent pass-through documents as `Box<serde_json::value::RawValue>` from input parsing through output dispatch, instead of materializing `serde_json::Value` in the common ingest path.

#### Scenario: NDJSON line is read
- **WHEN** the input reader consumes a valid NDJSON line
- **THEN** it returns an owned `Box<RawValue>` for that document
- **AND** it rejects invalid JSON before the document reaches any output

#### Scenario: CSV row is read
- **WHEN** the input reader consumes a valid CSV row
- **THEN** it converts the row into a JSON object string
- **AND** it returns that object as an owned `Box<RawValue>` without reparsing it into `Value`

### Requirement: Output pipeline consumes owned raw documents
The system SHALL pass owned raw documents into outputs so buffered senders can queue documents without cloning full JSON trees.

#### Scenario: Document is sent to an output
- **WHEN** the main ingest loop forwards a parsed document to an output
- **THEN** the sender interface consumes ownership of that `Box<RawValue>`
- **AND** no `Value` clone is required to place the document into an output buffer

### Requirement: Elasticsearch output applies bounded backpressure
The system SHALL decouple document production from Elasticsearch bulk flushing with a bounded async handoff and a controlled number of in-flight bulk requests.

#### Scenario: Elasticsearch output slows down
- **WHEN** the Elasticsearch sink cannot flush batches as quickly as input documents are read
- **THEN** document production experiences bounded backpressure through the output handoff
- **AND** the implementation does not create unbounded pending bulk-send work

### Requirement: Non-Elasticsearch outputs write raw JSON bytes
The system SHALL write raw JSON directly for stdout and file outputs.

#### Scenario: Document is written to stdout
- **WHEN** stdout is the selected output
- **THEN** the program writes the exact raw JSON document followed by a newline

#### Scenario: Document is written to a file
- **WHEN** a file output is selected
- **THEN** the program writes the exact raw JSON document followed by a newline
- **AND** it does not serialize the document through `serde_json::to_writer`

### Requirement: Elasticsearch bulk output preserves raw document buffering
The system SHALL buffer raw documents for Elasticsearch output and emit valid `_bulk` request bodies without requiring `Value` in the steady-state queue.

#### Scenario: Bulk queue flushes to Elasticsearch
- **WHEN** the Elasticsearch output flushes one or more buffered documents
- **THEN** it constructs a valid `_bulk` request body for `create` operations using the buffered raw JSON documents
- **AND** the implementation targets a batch size of `5,000` documents per bulk request
- **AND** Elasticsearch accepts the request without document-shape regressions

#### Scenario: Buffered documents are large
- **WHEN** the queue contains many large documents
- **THEN** the queue retains raw JSON payloads instead of cloned `Value` trees
- **AND** the implementation avoids additional whole-document copies beyond what is required to build the outbound request

### Requirement: Metadata remains compatible with raw payloads
The system SHALL keep any added output metadata separate from the raw document body so metadata support does not force pass-through payloads back into `Value`.

#### Scenario: Output metadata is added
- **WHEN** the implementation attaches metadata or bookkeeping information to queued documents
- **THEN** the raw JSON body remains stored as `Box<RawValue>`
- **AND** the metadata mechanism does not require reparsing the body into `serde_json::Value`

### Requirement: Large-file benchmark is recorded for the change
The system SHALL provide a documented before/after benchmark for this change using a localhost Elasticsearch target and a fixture of at least 100 MB.

#### Scenario: Benchmark evidence is captured
- **WHEN** the change is prepared for implementation or review
- **THEN** the change artifacts include the benchmark fixture size, target `localhost:9200`, elapsed time for the baseline and candidate runs, and document-count parity results
- **AND** the repository contains a checked-in benchmark in the test suite for repeating the measurement
- **AND** any benchmark harness caveats needed to explain transport or environment constraints are recorded alongside the numbers
