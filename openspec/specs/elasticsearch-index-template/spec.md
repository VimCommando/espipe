## Purpose

Define how `espipe` installs Elasticsearch composable index templates before bulk indexing.

## Requirements

### Requirement: Template option installs an Elasticsearch index template
The system SHALL accept `--template <path>` for Elasticsearch outputs and send the config file as a composable index template JSON request before sending any bulk document request.

#### Scenario: Template is installed before bulk indexing
- **WHEN** the user runs `espipe` with an Elasticsearch output and `--template template.json`
- **THEN** the system reads `template.json`
- **AND** it sends the parsed template to Elasticsearch as JSON before the first `_bulk` request
- **AND** it sends document batches only after Elasticsearch accepts the template request

#### Scenario: Default template name is derived from file name
- **WHEN** the user passes `--template ./templates/logs-docs.json`
- **THEN** the system sends the template request to `/_index_template/logs-docs`

#### Scenario: Template name is overridden
- **WHEN** the user passes `--template ./templates/logs-docs.json --template-name custom-template`
- **THEN** the system sends the template request to `/_index_template/custom-template`

#### Scenario: Empty template name is rejected
- **WHEN** the derived template name or `--template-name` value is empty
- **THEN** startup fails before any documents are sent
- **AND** the error explains that the template name must be non-empty

### Requirement: Template overwrite behavior is configurable
The system SHALL overwrite existing composable index templates by default and SHALL use Elasticsearch create-only template installation when `--template-overwrite=false`.

#### Scenario: Template overwrite defaults to true
- **WHEN** the user passes `--template template.json` without `--template-overwrite`
- **THEN** the system sends `PUT /_index_template/{template_name}` using overwrite semantics
- **AND** an existing template with the same name can be replaced if Elasticsearch authorizes the request

#### Scenario: Template overwrite is disabled
- **WHEN** the user passes `--template template.json --template-overwrite=false`
- **THEN** the system sends `POST /_index_template/{template_name}?create=true` with Elasticsearch create-only semantics
- **AND** the run fails if Elasticsearch reports that the template already exists
- **AND** no bulk document request is sent

### Requirement: Only composable index templates are supported
The system SHALL send template requests only to the Elasticsearch composable index template API.

#### Scenario: Template API path is used
- **WHEN** the user passes `--template template.json`
- **THEN** the system sends the request to the `/_index_template/{template_name}` API
- **AND** it does not send a request to the legacy `/_template/{template_name}` API

### Requirement: Template files must be valid supported template syntax
The system SHALL validate `.json`, `.jsonc`, `.json5`, `.yml`, and `.yaml` template files before sending them to Elasticsearch, and SHALL preserve strict JSON parsing for template files with other extensions for backwards compatibility.

#### Scenario: Template file is unreadable
- **WHEN** the user passes `--template` with a path that cannot be read
- **THEN** startup fails before any documents are sent
- **AND** the error identifies the template path
- **AND** the error is written to stderr

#### Scenario: Template file is invalid JSON
- **WHEN** the user passes `--template` with a file that is not valid JSON
- **THEN** startup fails before any documents are sent
- **AND** the error identifies the template path and JSON parse failure
- **AND** the error is written to stderr

#### Scenario: JSONC template contains comments
- **WHEN** the user passes `--template template.jsonc`
- **AND** the template contains C-style block comments
- **THEN** the system parses the template successfully
- **AND** it sends a valid JSON request body to Elasticsearch

#### Scenario: JSON5 template is provided
- **WHEN** the user passes `--template template.json5`
- **THEN** the system parses the template using JSON5-compatible syntax
- **AND** it sends a valid JSON request body to Elasticsearch

#### Scenario: YAML template file is provided
- **WHEN** the user passes `--template template.yml`
- **THEN** the system parses the YAML template successfully
- **AND** it sends a valid JSON request body to Elasticsearch

#### Scenario: Template file extension matching is case-insensitive
- **WHEN** the user passes `--template template.YAML`
- **THEN** the system treats the template file as YAML
- **AND** it sends a valid JSON request body to Elasticsearch

#### Scenario: Template file with unknown extension contains strict JSON
- **WHEN** the user passes `--template template.txt`
- **AND** the template file contains valid strict JSON
- **THEN** the system parses the template successfully
- **AND** it sends a valid JSON request body to Elasticsearch

#### Scenario: Commented template syntax is invalid
- **WHEN** the user passes `--template` with a `.jsonc` or `.json5` file that cannot be parsed
- **THEN** startup fails before any documents are sent
- **AND** the error identifies the template path and parse failure
- **AND** the error is written to stderr

### Requirement: Template index patterns are checked against target index
The system SHALL inspect template `index_patterns` and warn when no declared pattern matches the output target index name.

#### Scenario: Multi-target expression includes target index
- **WHEN** the output target index is `test3`
- **AND** the template JSON contains `index_patterns` with `test*`
- **THEN** the system treats the target index as matched
- **AND** it sends the template request without an index-pattern mismatch warning

#### Scenario: Multi-target exclusion removes target index
- **WHEN** the output target index is `test3`
- **AND** the template JSON contains `index_patterns` with `test*,-test3`
- **THEN** the system treats the target index as unmatched
- **AND** it emits an index-pattern mismatch warning

#### Scenario: Later include overrides earlier exclusion
- **WHEN** the output target index is `test3`
- **AND** the template JSON contains `index_patterns` with `test3*,-test3,test*`
- **THEN** the system treats the target index as matched
- **AND** it sends the template request without an index-pattern mismatch warning

#### Scenario: Index pattern matches target index
- **WHEN** the output target index is `logs-docs`
- **AND** the template JSON contains `index_patterns` that match `logs-docs`
- **THEN** the system sends the template request without an index-pattern mismatch warning

#### Scenario: Index pattern does not match target index
- **WHEN** the output target index is `logs-docs`
- **AND** the template JSON contains `index_patterns` that do not match `logs-docs`
- **THEN** the system emits a warning before sending documents
- **AND** the warning is written to stderr
- **AND** it does not fail solely because of the mismatch

#### Scenario: Index patterns cannot be checked
- **WHEN** the template JSON omits `index_patterns` or uses an unexpected `index_patterns` shape
- **THEN** the system emits a warning that the target index match could not be verified
- **AND** the warning is written to stderr
- **AND** Elasticsearch remains responsible for accepting or rejecting the template

#### Scenario: Index pattern syntax is invalid for local check
- **WHEN** the template JSON contains an `index_patterns` expression with a lone `-`
- **THEN** the system emits a warning that the target index match could not be verified
- **AND** the warning is written to stderr
- **AND** Elasticsearch remains responsible for accepting or rejecting the template

### Requirement: Template rejection aborts ingestion
The system SHALL abort the run when Elasticsearch rejects the template request.

#### Scenario: Elasticsearch rejects template
- **WHEN** Elasticsearch responds to the template request with a non-2xx status
- **THEN** the system fails the run
- **AND** no bulk document request is sent
- **AND** the error includes the response status and available Elasticsearch error details
- **AND** the error is written to stderr

#### Scenario: Template request cannot be completed
- **WHEN** the template request fails because of authentication, TLS, DNS, timeout, or transport error
- **THEN** the system fails the run
- **AND** no bulk document request is sent
- **AND** the error is written to stderr

### Requirement: Template argument failures occur before input access
The system SHALL validate template-related arguments and output compatibility before opening or reading input content.

#### Scenario: Template option is invalid
- **WHEN** the user provides invalid template-related arguments
- **THEN** startup fails before opening or reading input content
- **AND** the error is written to stderr

#### Scenario: Template option is incompatible with output
- **WHEN** the user provides template-related arguments with a non-Elasticsearch output
- **THEN** startup fails before opening or reading input content
- **AND** the error is written to stderr

### Requirement: Template option only applies to Elasticsearch outputs
The system SHALL reject template-related options when the selected output is not Elasticsearch.

#### Scenario: Template is used with file output
- **WHEN** the user passes `--template template.json` with a file output
- **THEN** startup fails before reading input documents
- **AND** the error explains that `--template` requires an Elasticsearch output
- **AND** the error is written to stderr

#### Scenario: Template is used with stdout output
- **WHEN** the user passes `--template template.json` with stdout output
- **THEN** startup fails before reading input documents
- **AND** the error explains that `--template` requires an Elasticsearch output
- **AND** the error is written to stderr

#### Scenario: Template name is used without Elasticsearch output
- **WHEN** the user passes `--template-name custom-template` with a file or stdout output
- **THEN** startup fails before reading input documents
- **AND** the error explains that template options require an Elasticsearch output
- **AND** the error is written to stderr

#### Scenario: Template name is used without template path
- **WHEN** the user passes `--template-name custom-template` without `--template`
- **THEN** startup fails before reading input documents
- **AND** the error explains that `--template-name` requires `--template`
- **AND** the error is written to stderr

#### Scenario: Template overwrite is used without template path
- **WHEN** the user passes `--template-overwrite=false` without `--template`
- **THEN** startup fails before reading input documents
- **AND** the error explains that `--template-overwrite` requires `--template`
- **AND** the error is written to stderr

### Requirement: Runs without template remain unchanged
The system SHALL preserve existing output behavior when `--template` is not provided.

#### Scenario: Elasticsearch output without template
- **WHEN** the user runs `espipe` with an Elasticsearch output and no `--template`
- **THEN** the system does not send an index template request
- **AND** document bulk indexing starts using the existing output flow
