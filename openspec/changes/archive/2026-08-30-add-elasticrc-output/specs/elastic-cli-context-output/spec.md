## Purpose

Allow espipe to select an Elasticsearch cluster and its credentials from an existing Elastic CLI context without copying or modifying `.elasticrc`.

## ADDED Requirements

### Requirement: Elastic CLI context output syntax
The system SHALL accept `.app:/index` as an Elasticsearch output using the active Elastic CLI context and `.context.app:/index` as an Elasticsearch output using a named context. The rightmost dot-separated segment before `:/` SHALL identify the application, and the preceding segments SHALL form the context name.

#### Scenario: Active context is selected
- **WHEN** the user supplies `.es:/logs-2026` as the final positional argument
- **THEN** the system selects the Elasticsearch service from the active Elastic CLI context
- **AND** the system uses `logs-2026` as the output index

#### Scenario: Named context is selected
- **WHEN** the user supplies `.production.es:/logs-2026` as the final positional argument
- **THEN** the system selects the Elasticsearch service from context `production`
- **AND** the system uses `logs-2026` as the output index

#### Scenario: Dotted context name is selected
- **WHEN** the user supplies `.production.us-west.elasticsearch:/logs-2026`
- **THEN** the system selects the Elasticsearch service from context `production.us-west`

### Requirement: Context outputs select Elasticsearch applications
The system SHALL accept `es` and `elasticsearch` as application aliases for context outputs. It SHALL reject other application values because espipe writes to the Elasticsearch bulk API.

#### Scenario: Canonical application name is accepted
- **WHEN** the user supplies `.production.elasticsearch:/logs-2026`
- **THEN** the system resolves the named context's Elasticsearch service

#### Scenario: Kibana application is rejected
- **WHEN** the user supplies `.production.kb:/logs-2026`
- **THEN** startup fails with an error that explains context outputs must select Elasticsearch

#### Scenario: Unknown application is rejected
- **WHEN** the user supplies `.production.search:/logs-2026`
- **THEN** startup fails with an error that identifies the unsupported application reference

### Requirement: Context output index is explicit
The system SHALL require context output targets to contain `:/` followed by a non-empty index. The system SHALL append that index to the resolved Elasticsearch service base path and SHALL discard any query or fragment from the configured service URL.

#### Scenario: Index is appended to a service base path
- **WHEN** context `production` resolves Elasticsearch to `https://example.com/elasticsearch/?ignored=true#fragment`
- **AND** the output is `.production.es:/logs-2026`
- **THEN** the bulk output URL is `https://example.com/elasticsearch/logs-2026`

#### Scenario: Index is missing
- **WHEN** the user supplies `.production.es:/`
- **THEN** startup fails with an error that explains the required `.context.app:/index` form

#### Scenario: Output uses an authority form
- **WHEN** the user supplies `.production.es://logs-2026`
- **THEN** startup fails with an error that explains the required `.context.app:/index` form

### Requirement: Elastic CLI config discovery is compatible
For a context output, the system SHALL first use `ELASTIC_CLI_CONFIG_FILE` when set. Otherwise, it SHALL discover the first readable `.elasticrc`, `.elasticrc.json`, `.elasticrc.yaml`, or `.elasticrc.yml` in the user's home directory using Elastic CLI discovery order. An active target SHALL select `current_context`; a named target SHALL select its explicit context.

#### Scenario: Explicit config environment variable is set
- **WHEN** `ELASTIC_CLI_CONFIG_FILE` names a readable supported config file
- **AND** the user supplies a context output
- **THEN** the system loads that file instead of a config from the user's home directory

#### Scenario: Active target uses current context
- **WHEN** the discovered config declares `current_context: production`
- **AND** the user supplies `.es:/logs-2026`
- **THEN** the system resolves the Elasticsearch service from context `production`

#### Scenario: Named target overrides current context
- **WHEN** the discovered config declares `current_context: development`
- **AND** the user supplies `.production.es:/logs-2026`
- **THEN** the system resolves the Elasticsearch service from context `production`

### Requirement: Context authentication is used by default
The system SHALL use the selected Elasticsearch service's API key, complete basic authentication pair, or lack of authentication when the user supplies no command-line authentication. An explicit `--apikey` or complete `--username` and `--password` pair SHALL take precedence over resolved context authentication.

#### Scenario: Context contains an API key
- **WHEN** the selected Elasticsearch service resolves API-key authentication
- **AND** the user supplies no authentication option
- **THEN** Elasticsearch requests use the resolved API key

#### Scenario: Context contains basic authentication
- **WHEN** the selected Elasticsearch service resolves a username and password
- **AND** the user supplies no authentication option
- **THEN** Elasticsearch requests use the resolved basic authentication

#### Scenario: Explicit authentication is provided
- **WHEN** the selected Elasticsearch service resolves authentication
- **AND** the user supplies a valid explicit authentication option
- **THEN** Elasticsearch requests use the explicit authentication

### Requirement: Context resolution failures stop the command
The system SHALL fail startup when a requested context output cannot be discovered, parsed, resolved, or converted into an Elasticsearch output. The error SHALL retain enough context to identify the failed config, context, service, index, or resolver without exposing a resolved secret.

#### Scenario: Config is not found
- **WHEN** the user supplies a context output and no supported Elastic CLI config can be found
- **THEN** startup fails with an Elastic CLI config-not-found error

#### Scenario: Named context is absent
- **WHEN** the user supplies `.production.es:/logs-2026`
- **AND** the loaded config has no `production` context
- **THEN** startup fails with an error naming the missing context

#### Scenario: Elasticsearch service is absent
- **WHEN** the selected context has no Elasticsearch service
- **THEN** startup fails with an error naming the missing service

#### Scenario: Credential resolver fails
- **WHEN** the selected Elasticsearch authentication uses a resolver that fails
- **THEN** startup fails without sending documents or printing the secret value

### Requirement: Elastic CLI config use is read-only
The system SHALL NOT create, update, or delete an Elastic CLI config file while resolving a context output. It SHALL resolve only the selected Elasticsearch service and SHALL keep resolved API keys and passwords out of debug and display output.

#### Scenario: Context output completes
- **WHEN** espipe successfully sends documents through a context output
- **THEN** the source Elastic CLI config file remains byte-for-byte unchanged

#### Scenario: Unselected service has a resolver
- **WHEN** the selected context has both Elasticsearch and Kibana services
- **AND** the Kibana service contains a resolver expression
- **THEN** resolving the Elasticsearch output does not evaluate the Kibana resolver

#### Scenario: Diagnostic logging is enabled
- **WHEN** a context authentication secret is resolved while diagnostic logging is enabled
- **THEN** logs and the runtime summary do not contain the resolved secret

### Requirement: Runtime summary reports processing time
The system SHALL start runtime summary timing after input and output initialization. The reported duration SHALL include document reads, sends, and output close, but SHALL exclude Elastic CLI config resolution and time spent waiting for credential authorization.

#### Scenario: Credential resolution waits for authorization
- **WHEN** a context credential resolver blocks before the Elasticsearch output is ready
- **THEN** the final runtime summary excludes that blocked duration
- **AND** the summary reports only the subsequent document processing time
