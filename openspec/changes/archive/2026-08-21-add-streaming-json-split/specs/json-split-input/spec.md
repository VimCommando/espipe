## Purpose

Define how users incrementally select and split a JSON array or object into individual documents while retaining bounded processing and existing output behavior.

## ADDED Requirements

### Requirement: JSON splitting is explicitly selected
The system SHALL provide a `--split <json_pointer>` option that interprets the single input source as one JSON document and splits the selected collection, without changing default JSON or NDJSON parsing.

#### Scenario: Root split is selected
- **WHEN** the user runs `espipe --split / games.json output.ndjson`
- **THEN** the system selects the root JSON value for splitting
- **AND** it does not apply line-delimited JSON record boundaries

#### Scenario: Standard JSON input is used without split mode
- **WHEN** the user provides a `.json` or `.ndjson` input without `--split`
- **THEN** the system preserves the existing line-delimited and whole-object compatibility behavior

#### Scenario: Split mode receives multiple inputs
- **WHEN** the user combines `--split` with more than one input source
- **THEN** startup fails before any document is sent
- **AND** the error states that split mode accepts exactly one input source

### Requirement: Split paths select nested collections
The system SHALL resolve the split path as JSON Pointer tokens from the root, with `/` as a root alias and one trailing slash ignored for non-root paths.

#### Scenario: Wrapped collection is selected
- **WHEN** the input contains `{"hits":[{"name":"Alpha"},{"name":"Beta"}]}`
- **AND** the user passes `--split /hits/`
- **THEN** the system splits the array stored beneath `hits`
- **AND** the `hits` wrapper is not present in emitted documents

#### Scenario: Trailing slash is omitted
- **WHEN** the user passes `--split /hits`
- **THEN** the selected value is the same as for `--split /hits/`

#### Scenario: Final empty-name member is requested
- **WHEN** the user passes a path with two trailing slashes such as `--split /hits//`
- **THEN** startup fails with an error stating that final empty-name members are not supported

#### Scenario: Escaped object key is selected
- **WHEN** a path token contains `~1` or `~0`
- **THEN** the system resolves those sequences as `/` or `~` respectively

#### Scenario: Pointer traverses an array
- **WHEN** an intermediate selected value is an array and the next token is a valid zero-based index
- **THEN** the system continues pointer evaluation through that array element

#### Scenario: Pointer does not resolve
- **WHEN** a path token names a missing object member, uses an invalid array index, or traverses a scalar
- **THEN** ingestion fails with an error identifying the split path and failing token

### Requirement: Selected objects stream property values as documents
The system SHALL emit each property value of a selected JSON object as one JSON object document, preserve its JSON values without numeric precision loss, add the property name as a string-valued `id` field, and make no guarantee about output order.

#### Scenario: Root object is split
- **WHEN** the selected object is `{"10":{"name":"Alpha"},"20":{"name":"Beta"}}`
- **THEN** the system emits `{"id":"10","name":"Alpha"}` and `{"id":"20","name":"Beta"}`
- **AND** either document may be emitted first

#### Scenario: Numeric-looking object key is used
- **WHEN** a selected object property name is `730`
- **THEN** the emitted document contains `"id":"730"`

#### Scenario: Map values contain arbitrary-precision numbers
- **WHEN** a selected object property contains a valid JSON number beyond native integer or floating-point precision
- **THEN** the emitted document preserves that number without rounding

#### Scenario: Object property already contains an id field
- **WHEN** a selected object property value already contains an `id` field
- **THEN** ingestion fails when that property is reached
- **AND** the error identifies the property key and conflicting `id` field
- **AND** the system does not overwrite the existing value

#### Scenario: Selected object is empty
- **WHEN** the selected collection is an empty object
- **THEN** the system emits zero documents and completes successfully

### Requirement: Selected arrays stream elements as documents
The system SHALL emit each element of a selected JSON array as one JSON object document without adding a synthetic identifier or losing numeric precision and SHALL NOT promise source-order preservation.

#### Scenario: Root array is split
- **WHEN** the selected array is `[{"id":"10","name":"Alpha"},{"id":"20","name":"Beta"}]`
- **THEN** the system emits both objects in an unspecified order
- **AND** it preserves each object's fields and JSON types

#### Scenario: Array objects do not contain ids
- **WHEN** a selected array contains object elements without an `id` field
- **THEN** the system emits those objects unchanged
- **AND** it does not add an array-index identifier

#### Scenario: Array values contain arbitrary-precision numbers
- **WHEN** a selected array object contains a valid JSON number beyond native integer or floating-point precision
- **THEN** the emitted document preserves that number without rounding

#### Scenario: Selected array is empty
- **WHEN** the selected collection is an empty array
- **THEN** the system emits zero documents and completes successfully

### Requirement: Split documents must be JSON objects
The system SHALL reject any selected map value or array element that cannot be represented directly as a JSON object document.

#### Scenario: Object property value is not an object
- **WHEN** a selected object's property value is an array, scalar, or null
- **THEN** ingestion fails when that property is reached
- **AND** the error identifies its property key

#### Scenario: Array element is not an object
- **WHEN** a selected array element is an array, scalar, or null
- **THEN** ingestion fails when that element is reached
- **AND** the error identifies its zero-based array index

#### Scenario: Selected value is not a collection
- **WHEN** the split path resolves to a scalar or null
- **THEN** ingestion fails with an error stating that the selected value must be an array or object

### Requirement: JSON splitting remains bounded
The system SHALL navigate and deserialize split documents incrementally without materializing the complete input, skipped wrapper subtrees, or an unbounded collection of emitted documents in memory.

#### Scenario: Large selected collection is ingested
- **WHEN** a selected array or object contains more documents than the configured Elasticsearch bulk batch size
- **THEN** documents flow incrementally through the existing bounded output pipeline
- **AND** split-input memory is bounded independently of the collection's total length

#### Scenario: Batches complete at different speeds
- **WHEN** parallel split workers complete document batches in a different order from the source
- **THEN** each valid selected child is emitted exactly once
- **AND** the system does not buffer completed batches to restore source order

#### Scenario: Output applies backpressure
- **WHEN** the selected output consumes documents more slowly than the split parser produces them
- **THEN** parsing waits on a bounded handoff
- **AND** it does not accumulate an unbounded queue of parsed documents

#### Scenario: First batch precedes end of input
- **WHEN** the selected collection has a full valid worker batch followed by substantial remaining content
- **THEN** documents from that batch can reach the output before the complete input has been read

### Requirement: Split documents use existing outputs
The system SHALL pass each emitted split document through the same output dispatch used by other input formats.

#### Scenario: Split documents are written to NDJSON
- **WHEN** split mode targets stdout or a local NDJSON output
- **THEN** each selected child is written as one JSON line with the required map-key transformation, if any

#### Scenario: Split documents are sent to Elasticsearch
- **WHEN** split mode targets Elasticsearch
- **THEN** documents use the configured bulk action, batch size, and maximum in-flight request limit
- **AND** configured ingest pipelines and index templates retain their existing behavior

### Requirement: Invalid split inputs produce contextual errors
The system SHALL stop split ingestion on invalid pointer syntax, unresolved paths, malformed JSON, trailing JSON content, or invalid documents and SHALL report the source plus available path and child context.

#### Scenario: JSON is malformed
- **WHEN** split mode encounters malformed or trailing JSON content
- **THEN** ingestion fails with an error that identifies the input source and JSON parse location

#### Scenario: Error follows valid documents
- **WHEN** malformed or invalid content occurs after valid selected children
- **THEN** the system cancels new batch work after observing the error
- **AND** documents from concurrently in-flight batches may already have been emitted
- **AND** the diagnostic does not claim that already-sent output was rolled back
