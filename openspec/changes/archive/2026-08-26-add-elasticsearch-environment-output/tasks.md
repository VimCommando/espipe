## 1. Environment output implementation

- [x] 1.1 Reserve `env:/<index>` for environment-backed Elasticsearch output and return `es` and `elasticsearch` to configured-host resolution.
- [x] 1.2 Load missing Elastic connection settings from `.env` without replacing process environment values, and report missing or invalid URLs before ingestion.
- [x] 1.3 Restrict environment API-key fallback to `env:/` output while preserving explicit command-line authentication precedence.

## 2. Verification and documentation

- [x] 2.1 Cover environment URI syntax, URL construction, and authentication precedence with unit tests.
- [x] 2.2 Cover process environment precedence, `.env` fallback and parse failure, non-environment scoping, and missing URL failure with CLI tests.
- [x] 2.3 Document `env:/` usage, `.env` lookup, migration from the former schemes, and the new dependency.
