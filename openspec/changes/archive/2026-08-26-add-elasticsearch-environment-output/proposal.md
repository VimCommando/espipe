## Why

The `es:/` and `elasticsearch:/` output schemes do not say that espipe resolves their connection settings from environment variables. They also prevent users from assigning those names to configured hosts. An explicit `env:/` target makes the configuration source clear and leaves ordinary host aliases available.

## What changes

- Add `env:/<index>` as the Elasticsearch output form backed by `ELASTIC_ES_URL` and the optional `ELASTIC_ES_API_KEY`.
- Load missing environment settings from the nearest `.env` file without replacing values already present in the process environment.
- Fail with a clear error when `ELASTIC_ES_URL` remains unset or is not an absolute HTTP or HTTPS URL.
- Preserve explicit command-line authentication precedence over `ELASTIC_ES_API_KEY`.
- **BREAKING**: Stop reserving `es:/` and `elasticsearch:/`; resolve them as configured host names instead.

## Capabilities

### New capabilities

- `elasticsearch-environment-output`: Defines the environment-backed output URI, setting precedence, URL validation, authentication precedence, and configured-host namespace behavior.

### Modified capabilities

None.

## Impact

- Affects output URI dispatch and environment authentication handling in `src/main.rs` and `src/output/mod.rs`.
- Adds the `dotenvy` runtime dependency.
- Adds CLI coverage for process environment, `.env`, and missing-setting behavior.
- Changes commands that used `es:/` or `elasticsearch:/` to use `env:/`.
