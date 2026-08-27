## Purpose

Define how an `env:/` Elasticsearch output resolves connection settings while preserving configured host names and explicit command-line authentication.

## Requirements

### Requirement: Environment output uses an explicit URI form

The system SHALL reserve `env:/<index>` for an Elasticsearch output whose connection settings come from the process environment or `.env`. The URI SHALL contain one slash after `env:` and a non-empty index path. The system SHALL NOT reserve `es` or `elasticsearch` for environment-backed output.

#### Scenario: Valid environment output is provided

- **WHEN** the user provides `env:/logs` as the output
- **THEN** the system selects environment-backed Elasticsearch output
- **AND** it uses `logs` as the target index

#### Scenario: Environment output omits the required index

- **WHEN** the user provides `env:/` as the output
- **THEN** startup fails with an error that identifies `env:/index` as the required form

#### Scenario: Environment output uses an authority or omits the slash

- **WHEN** the user provides `env://logs` or `env:logs` as the output
- **THEN** startup fails with an error that identifies `env:/index` as the required form

#### Scenario: Former environment scheme is used

- **WHEN** the user provides an output whose scheme is `es` or `elasticsearch`
- **THEN** the system resolves that scheme as a configured host name
- **AND** it does not read Elastic environment settings for that output

### Requirement: Environment settings use deterministic precedence

For an `env:/` output, the system SHALL use values already present in the process environment. For each missing setting, it SHALL search the working directory and its ancestors for the nearest `.env` file and load the setting from that file. A `.env` value SHALL NOT replace a value already present in the process environment.

#### Scenario: Process environment contains the URL

- **WHEN** `ELASTIC_ES_URL` is present in the process environment
- **AND** `.env` contains a different `ELASTIC_ES_URL`
- **THEN** the system uses the value from the process environment

#### Scenario: Dotenv supplies a missing URL

- **WHEN** `ELASTIC_ES_URL` is absent from the process environment
- **AND** the nearest `.env` file defines `ELASTIC_ES_URL`
- **THEN** the system uses the value from `.env`

#### Scenario: Dotenv file is malformed

- **WHEN** the nearest `.env` file cannot be parsed
- **THEN** startup fails with an error that identifies `.env` as unreadable

#### Scenario: Non-environment output is selected

- **WHEN** the output does not use the `env` scheme
- **THEN** the system does not load `.env` for Elasticsearch connection settings

### Requirement: Environment URL is required and valid

An `env:/` output SHALL require `ELASTIC_ES_URL` after environment and `.env` resolution. The value SHALL be an absolute `http://` or `https://` URL with a host. The system SHALL append the output index to any existing base path and SHALL discard query and fragment components from the configured URL.

#### Scenario: URL remains unset

- **WHEN** neither the process environment nor `.env` defines `ELASTIC_ES_URL`
- **THEN** startup fails with an error that names `ELASTIC_ES_URL`

#### Scenario: URL uses an unsupported or relative form

- **WHEN** the resolved `ELASTIC_ES_URL` is relative, lacks a host, or uses a scheme other than HTTP or HTTPS
- **THEN** startup fails before sending documents

#### Scenario: URL contains a base path

- **WHEN** `ELASTIC_ES_URL` is `https://example.com/elasticsearch/?ignored=true#fragment`
- **AND** the output is `env:/logs`
- **THEN** the Elasticsearch output URL is `https://example.com/elasticsearch/logs`

### Requirement: Explicit authentication takes precedence

For an `env:/` output, the system SHALL use `ELASTIC_ES_API_KEY` from the process environment or `.env` when the user supplies no authentication option. An explicit `--apikey` or complete `--username` and `--password` pair SHALL take precedence over `ELASTIC_ES_API_KEY`.

#### Scenario: Environment API key is the only authentication setting

- **WHEN** `ELASTIC_ES_API_KEY` resolves from the process environment or `.env`
- **AND** the user supplies no authentication option
- **THEN** the system authenticates with the resolved API key

#### Scenario: Explicit API key is provided

- **WHEN** the user supplies `--apikey`
- **AND** `ELASTIC_ES_API_KEY` is also set
- **THEN** the system authenticates with the explicit API key

#### Scenario: Explicit basic authentication is provided

- **WHEN** the user supplies `--username` and `--password`
- **AND** `ELASTIC_ES_API_KEY` is also set
- **THEN** the system authenticates with the explicit basic credentials
