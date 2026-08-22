## Purpose

Define explicit Elasticsearch bulk actions and stable document identity for repeatable ingestion of multi-source local document collections, while preserving an easy single-source default.

## ADDED Requirements

### Requirement: Supported bulk actions are explicit

The system SHALL accept `create`, `index`, `update`, and `upsert` as Elasticsearch bulk actions, and SHALL use `index` when the user does not provide `--action`.

#### Scenario: Action is omitted

- **WHEN** the user sends documents to an Elasticsearch output without `--action`
- **THEN** the system emits `index` bulk operations

#### Scenario: Action is selected explicitly

- **WHEN** the user passes `--action create`, `--action index`, `--action update`, or `--action upsert`
- **THEN** the system uses the selected action when constructing each bulk operation

### Requirement: Create and index operations preserve the selected bulk operation

The system SHALL emit a `create` metadata line for `create` and an `index` metadata line for `index`, followed by the source document line. When an ID is available, the metadata line SHALL include it as `_id`; otherwise the metadata line SHALL omit `_id` and Elasticsearch SHALL assign the ID.

#### Scenario: Create uses an available ID

- **WHEN** the user selects `create` and the document has an explicit or generated ID
- **THEN** the bulk metadata line contains `{"create":{"_id":"<id>"}}`
- **AND** the following source line does not contain the transport-only top-level `_id` field

#### Scenario: Index uses an available ID

- **WHEN** the user selects `index` and the document has an explicit or generated ID
- **THEN** the bulk metadata line contains `{"index":{"_id":"<id>"}}`
- **AND** the following source line does not contain the transport-only top-level `_id` field

#### Scenario: ID generation is disabled

- **WHEN** the user selects `create` or `index`
- **AND** the document has no explicit `_id`
- **AND** generated IDs are disabled
- **THEN** the bulk metadata line omits `_id`
- **AND** the source document is sent without a generated identifier

### Requirement: Update and upsert payloads express user intent to Elasticsearch

The system SHALL emit an `update` metadata line with the selected document ID for both `update` and `upsert`. The following payload line SHALL contain the source document under `doc` after removing the transport-only top-level `_id` field. The `upsert` payload SHALL additionally contain `doc_as_upsert: true`; the `update` payload SHALL not enable `doc_as_upsert`.

#### Scenario: Update targets an existing document

- **WHEN** the user selects `update` and the document has an explicit or generated string ID
- **THEN** the system emits an `update` metadata line containing that ID
- **AND** the following line contains a `doc` object
- **AND** the payload does not contain `doc_as_upsert: true`

#### Scenario: Upsert allows Elasticsearch to create a missing document

- **WHEN** the user selects `upsert` and the document has an explicit or generated string ID
- **THEN** the system emits an `update` metadata line containing that ID
- **AND** the following line contains a `doc` object
- **AND** the payload contains `doc_as_upsert: true`

### Requirement: Explicit document IDs take precedence and must be strings

The system SHALL use a document's top-level `_id` value when present instead of generating an ID. A top-level `_id` used as bulk metadata SHALL be a string, and SHALL be removed from the source document or update `doc` payload.

#### Scenario: Explicit ID overrides a generated file ID

- **WHEN** a file document contains a top-level string `_id`
- **AND** generated IDs are enabled
- **THEN** the bulk operation uses the explicit `_id`
- **AND** the system does not replace it with a generated ID

#### Scenario: Non-string explicit ID is rejected

- **WHEN** a document contains a top-level `_id` whose value is not a string
- **THEN** the action fails validation
- **AND** the system does not silently generate a replacement ID

### Requirement: Local-source IDs are deterministic by bundle, path, and discriminator

The system SHALL generate an ID for each eligible local-file document by applying a stable encoding to the bundle identifier, the source file's working-directory-relative path, and, when needed, a document discriminator. The generated ID SHALL not depend on file contents, modification time, absolute checkout path, or checkout-specific metadata.

The discriminator SHALL be a stable per-source value: a typed source key for a split object, a zero-based array index for a split array, a zero-based record ordinal for an ordinary multi-document stream, or `0` for a source whose generated identity is explicitly enabled and which emits one document. The discriminator SHALL be transport identity and need not be present in the document body.

The bundle identifier SHALL be the repository directory name when the working path is inside a tracked Git repository. When it is not inside a tracked Git repository, the bundle identifier SHALL be the parent directory name. The relative path SHALL use the file path beneath the working directory, including its extension.

The stable encoding SHALL be the first 128 bits of the SHA-256 digest of the canonical key, encoded as URL-safe Base64 without padding. Generated IDs SHALL therefore be 22 characters long. The encoding SHALL not include the absolute checkout path or document contents.

#### Scenario: Same file identity produces the same ID

- **WHEN** the same bundle identifier and relative source path are ingested more than once
- **THEN** the system generates the same `_id` each time
- **AND** changing the file contents does not change that `_id`

#### Scenario: Separate checkouts retain file IDs

- **WHEN** the same bundle is checked out again with the same bundle identifier
- **AND** a file has the same working-directory-relative path
- **THEN** the generated `_id` matches the ID from the earlier checkout

#### Scenario: Generated IDs are enabled for multi-source inputs by default

- **WHEN** the user imports a multi-source local file input without passing `--generate-id`
- **AND** the document has no explicit `_id`
- **THEN** the system generates an ID from the bundle, relative source path, and document discriminator

#### Scenario: Generated IDs are disabled for single-source inputs by default

- **WHEN** the user imports a single-source local file input without passing `--generate-id=true`
- **AND** the document has no explicit `_id`
- **THEN** the system does not generate an ID

#### Scenario: Single-source ID generation can be explicitly enabled

- **WHEN** the user imports a single-source local file input with `--generate-id=true`
- **AND** the document has no explicit `_id`
- **THEN** the system generates an ID from the bundle, relative source path, and document discriminator

#### Scenario: Split object keys provide document identity

- **WHEN** a local file is split at an object collection
- **AND** a source object has key `alpha`
- **THEN** the generated ID discriminator for that document includes the typed key `alpha`

#### Scenario: Split array positions provide document identity

- **WHEN** a local file is split at an array collection
- **AND** a source object is at array position `3`
- **THEN** the generated ID discriminator for that document includes the typed index `3`

#### Scenario: Generated IDs use compact 128-bit encoding

- **WHEN** a local file document receives a generated ID
- **THEN** the ID is a 22-character URL-safe Base64 value without padding
- **AND** it represents the first 128 bits of the SHA-256 digest of the canonical bundle, path, and discriminator key

### Requirement: Generated ID mode is explicit and cardinality-aware

The system SHALL accept `--generate-id=true` and `--generate-id=false`. When the option is omitted, the system SHALL enable generated IDs for multi-source local file inputs and disable them for single-source local file inputs. Non-file input forms SHALL not receive generated IDs from this feature.

#### Scenario: File ID generation is disabled for update or upsert

- **WHEN** the user passes `--generate-id=false`
- **AND** selects `update` or `upsert`
- **AND** a local file document has no explicit string `_id`
- **THEN** validation fails before that document is emitted as a bulk operation

#### Scenario: File ID generation is disabled for create or index

- **WHEN** the user passes `--generate-id=false`
- **AND** selects `create` or `index`
- **AND** a local file document has no explicit `_id`
- **THEN** the operation omits `_id` and lets Elasticsearch assign it

#### Scenario: Single-source update requires an explicit ID by default

- **WHEN** the user selects `update` or `upsert` for a single-source input
- **AND** the user does not pass `--generate-id=true`
- **AND** the document has no explicit string `_id`
- **THEN** validation fails before that document is emitted as a bulk operation

### Requirement: Bulk responses recognize update success and no-op results

The system SHALL parse bulk response items for `create`, `index`, and `update` operations. A successful update, including an Elasticsearch `noop` result, SHALL count as a successful operation and SHALL not be reported as an item failure.

#### Scenario: Update response is successful

- **WHEN** Elasticsearch returns a successful `update` bulk item
- **THEN** the system counts the item as successful

#### Scenario: Update response is a no-op

- **WHEN** Elasticsearch returns an `update` bulk item with a `noop` result
- **THEN** the system counts the item as successful
- **AND** the system does not log it as a bulk item error

#### Scenario: Update response contains an item error

- **WHEN** Elasticsearch returns an error for an `update` bulk item
- **THEN** the system reports the item failure using the existing bulk error handling behavior

#### Scenario: Bulk item errors without nested causes are reported

- **WHEN** Elasticsearch returns a bulk item error with top-level `type` and `reason` fields
- **AND** the error does not include a `caused_by` object
- **THEN** the system parses the bulk response successfully
- **AND** the system reports the item failure using the top-level error details

#### Scenario: Bulk error summaries remain bounded

- **WHEN** a bulk response contains many item failures with repeated or position-specific details
- **THEN** the system aggregates equivalent normalized errors
- **AND** the warning summary is bounded in number and total length
- **AND** the warning identifies when additional summaries are omitted
