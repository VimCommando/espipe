## 1. Embedded template catalog

- [x] 1.1 Add `rust-embed` to `Cargo.toml` and include the template asset directory in the published crate file list.
- [x] 1.2 Add an embedded-template module that resolves logical selector names, reads each asset's default Elasticsearch template name, and enumerates available bundled templates.
- [x] 1.3 Add `assets/templates/_okf.yaml` with OKF v0.2 metadata, an `espipe` revision, and `open-knowledge-format` as its default installation name.
- [x] 1.4 Add unit coverage proving `_okf` is available from the compiled asset catalog and unknown selectors report the available names.

## 2. OKF mapping contract

- [x] 2.1 Explicitly map core OKF metadata, searchable body fields, and `origin` identity fields with their specified Elasticsearch types.
- [x] 2.2 Explicitly map all provenance, trust, lifecycle, and Attested Computation structures, including nested source, verifier, and parameter records.
- [x] 2.3 Disable automatic date detection and add a final dynamic template that maps undeclared strings once as bounded `keyword` fields.
- [x] 2.4 Add mapping tests that account for every official OKF v0.2 field and reject automatic `text` plus `keyword` mappings for extension strings.
- [x] 2.5 Add a representative OKF document fixture covering single-object `verified`, list `verified`, provenance usage windows, lifecycle metadata, and attested computation fields.

## 3. Template source resolution

- [x] 3.1 Refactor template configuration to distinguish file-backed paths from leading-underscore bundled selectors while preserving existing CLI validation timing.
- [x] 3.2 Parse bundled YAML through the shared parsed-template representation without filesystem access.
- [x] 3.3 Preserve all existing JSON, JSONC, JSON5, YAML, naming, pipeline compatibility, and index-pattern warning behavior for file-backed templates.
- [x] 3.4 Add CLI and preflight tests for `_okf`, unknown bundled selectors, underscore-containing file paths, and unchanged non-Elasticsearch rejection.

## 4. Shared template maintenance

- [x] 4.1 Resolve the selected Elasticsearch template name from the bundled asset default or `--template-name`, with `_okf` defaulting to `open-knowledge-format`.
- [x] 4.2 Add exact-name template lookup for the selected name and parse the Elasticsearch `index_templates` response into one stored composable template body.
- [x] 4.3 When the selected template is absent, append the target index to the bundled asset and create it under the default or overridden name before bulk ingestion.
- [x] 4.4 When the shared template exists and lacks the exact target index, append it to the stored `index_patterns` and update the same template without changing other fields.
- [x] 4.5 Skip the update when the exact target index is already listed, while preserving file-backed template behavior.
- [x] 4.6 Add request-capture tests for lookup failures, default and overridden names, initial creation, append updates, exact-value no-ops, wildcard-only patterns, malformed stored templates, create-only branches, and preserved stored fields.
- [x] 4.7 Document the last-write-wins limitation when concurrent processes update the shared template for different targets.
- [x] 4.8 Add a binary-level test that runs outside the source tree and proves `_okf` resolves without local assets or network lookup.

## 5. Documentation and verification

- [x] 5.1 Update CLI help and README template documentation with bundled selector syntax, default and overridden template names, merge behavior, the OKF v0.2 version pin, mapping defaults, and an OKF ingestion example.
- [x] 5.2 Update the repository `espipe` skill so ingestion requests can select `--template _okf` accurately.
- [x] 5.3 Add an Unreleased changelog entry for bundled OKF template support.
- [x] 5.4 Run formatting, unit tests, index-template integration tests, strict OpenSpec validation, and package-content verification.
