## Purpose

Define the Elasticsearch mappings and dynamic mapping policy supplied by the bundled template for OKF documents ingested through `espipe`.

## ADDED Requirements

### Requirement: Bundled OKF template targets the current supported specification
The system SHALL bundle an Elasticsearch composable index template named `_okf` whose metadata identifies Open Knowledge Format v0.2 and a template revision controlled by `espipe`.

#### Scenario: Bundled template version is inspectable
- **WHEN** the `_okf` asset is decoded as a composable index template
- **THEN** its `_meta` identifies OKF specification version `0.2`
- **AND** its `_meta` contains an `espipe` template revision

### Requirement: Official OKF scalar and list metadata fields have explicit mappings
The `_okf` template SHALL explicitly map `content.type`, `content.title`, `content.description`, `content.resource`, `content.tags`, `content.okf_version`, `content.status`, `content.stale_after`, `content.runtime`, and `content.computation`. Categorical values, identifiers, paths, URIs, and tags SHALL be `keyword`; prose SHALL be `text`; and lifecycle instants SHALL be `date`.

#### Scenario: Core concept metadata mappings are installed
- **WHEN** Elasticsearch installs a materialized `_okf` template
- **THEN** `content.type`, `content.resource`, `content.tags`, `content.okf_version`, `content.status`, `content.runtime`, and `content.computation` are mapped as `keyword`
- **AND** `content.title` and `content.description` are mapped as `text`
- **AND** `content.stale_after` is mapped as `date`

#### Scenario: Markdown content is searchable
- **WHEN** `espipe` emits OKF document content in `content.body` or `content.markdown`
- **THEN** the template maps both fields as `text`

### Requirement: Official OKF structured metadata fields have explicit mappings
The `_okf` template SHALL explicitly map the complete v0.2 structures for `content.sources`, `content.usage_window`, `content.generated`, `content.verified`, `content.parameters`, `content.executor`, and `content.attester`, including every child field defined by the specification.

#### Scenario: Provenance fields preserve source associations
- **WHEN** an OKF concept contains `sources`
- **THEN** `content.sources` is mapped as `nested`
- **AND** each source's `id`, `resource`, and `author` are `keyword`
- **AND** each source's `title` is `text`
- **AND** each source's `usage_count` is `long`
- **AND** each source's `last_modified`, `usage_window.from`, and `usage_window.to` are `date`
- **AND** top-level `content.usage_window.from` and `content.usage_window.to` are `date`

#### Scenario: Trust and lifecycle structures are mapped
- **WHEN** an OKF concept contains generation or verification metadata
- **THEN** `content.generated.by` is `keyword` and `content.generated.at` is `date`
- **AND** `content.verified` is `nested` with `by` as `keyword` and `at` as `date`
- **AND** Elasticsearch accepts either the specification's single-object or list representation of `verified`

#### Scenario: Attested computation structures are mapped
- **WHEN** an OKF Attested Computation contains contract metadata
- **THEN** `content.parameters` is `nested` with `name` and `type` as `keyword` and `required` as `boolean`
- **AND** `content.executor.resource` and every `content.executor.receipt` value are `keyword`
- **AND** `content.attester.resource` is `keyword`

### Requirement: Unknown strings do not receive automatic text and keyword multifields
The `_okf` template SHALL disable automatic date detection and SHALL use a final string dynamic template that maps undeclared string fields once as `keyword`, without an automatically generated `text` or `keyword` multifield.

#### Scenario: Producer extension string is mapped once
- **WHEN** an OKF concept contains an undeclared producer field such as `content.owner`
- **THEN** Elasticsearch dynamically maps `content.owner` as `keyword`
- **AND** the mapping does not add a `text` representation or a multifield

#### Scenario: Date-like extension remains a string
- **WHEN** an undeclared producer string resembles a date
- **THEN** automatic date detection does not map it as `date`
- **AND** the string dynamic template maps it as `keyword`

### Requirement: Espipe origin metadata has stable mappings
The `_okf` template SHALL explicitly map `origin.scheme`, `origin.path`, and `origin.filename` as `keyword` so file identity fields remain filterable and do not receive text multifields.

#### Scenario: Local file origin is mapped for filtering
- **WHEN** `espipe` indexes an OKF document with local-file origin metadata
- **THEN** `origin.scheme`, `origin.path`, and `origin.filename` are mapped as `keyword`
