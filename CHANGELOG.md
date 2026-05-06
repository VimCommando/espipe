# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-05-06

### Added

- Added gzip-compressed local `.csv.gz` and `.ndjson.gz` input support.
- Added gzip-compressed local `.ndjson.gz` file output support.
- Added compressed NDJSON fixture coverage for localhost Elasticsearch ingestion.
- Added YAML file support for Elasticsearch ingest pipeline and composable index template configuration.
- Added YAML example configs for a Steam games Elasticsearch ingest pipeline and index template.
- Added `--version` CLI output.
- Added Toon input support for local and remote `.toon` sources, including `---`-separated documents and top-level tabular object arrays.

### Fixed

- Rejected unsupported gzip input and output suffixes consistently before ingestion or file creation.
- Flushed gzip-compressed file outputs on close so completed `.ndjson.gz` files are readable immediately.

## [0.3.0] - 2026-04-29

### Added

- Added local file-document imports with shell-expanded file lists and recursive glob discovery.
- Added format-aware file imports for Markdown frontmatter, plain text, YAML, JSON, NDJSON, and JSONL inputs.
- Added configurable file content fields with conditional `file.path` and `file.name` metadata for multi-file imports.
- Added Elasticsearch composable index template installation with JSON, JSONC, and JSON5 template file support.
- Added configurable index template overwrite behavior and index-pattern validation warnings.
- Added Elasticsearch ingest pipeline installation with bulk request pipeline targeting.
- Added pipeline/template compatibility checks, including support for template-defined default pipelines and disabling defaults with `_none`.

### Changed

- Changed positional input parsing so multiple local inputs can be ingested before the final output URI.
- Ensured template and pipeline preflight failures abort before bulk ingestion starts.
- Preserved existing CSV, JSON, NDJSON, stdin, HTTPS, file, and stdout behavior when the new options are omitted.

## [0.2.0] - 2026-04-20

### Added

- Added a RawValue-based document pipeline to preserve NDJSON records without reparsing and reserializing them during output.
- Added optional localhost Elasticsearch integration coverage and file-output integration tests.
- Added localhost benchmark coverage, including generated nginx access log fixtures for manual benchmarking.
- Added crates.io publication metadata including homepage, documentation, keywords, categories, and curated package contents.
- Added an Apache-2.0 `LICENSE.md`.
- Added publish-focused documentation and installation guidance.

### Changed

- Reworked Elasticsearch bulk output handling around the RawValue pipeline and updated the CLI/output path to use it.
- Improved bulk ingestion resilience with retry handling for HTTP `429 Too Many Requests`.
- Fixed CLI authentication and argument validation issues discovered during the `0.2.0` development cycle.
- Switched the crate license from `AGPL-3.0` to `Apache-2.0`.
- Removed the public library target so the crate is documented and shipped as a CLI-only package.
- Upgraded dependency versions across the crate for the publication pass.

## [0.1.3] - 2026-01-28

### Added

- Added bulk action selection for Elasticsearch outputs with support for `create`, `index`, and `update`.
- Added update-mode validation requiring a string `_id` on each document.
- Added dev-dependency support for expanded test coverage.

### Changed

- Upgraded to the `elasticsearch` `9.1.0-alpha.1` client.
- Updated `fluent-uri` and other crate dependencies.

## [0.1.2] - 2026-01-28

### Changed

- Moved the crate to Rust edition 2024.
- Declared a minimum supported Rust version of `1.88`.
- Normalized dependency requirements to caret constraints.

## [0.1.1] - 2024-12-26

### Changed

- Upgraded to the newer `elasticsearch` `8.17.0-alpha.1` client.
- Removed the temporary crates.io patch override once gzip request compression support was available in the upstream client.

## [0.1.0] - 2024-11-26

### Added

- Initial release of `espipe` as a command-line bulk ingestion tool for Elasticsearch.
- Added NDJSON file and `stdin` ingestion support.
- Added multithreaded Elasticsearch bulk output handling.
- Added optional request body gzip compression support.
- Added CSV input support with row-to-JSON conversion.
- Added project metadata including description, repository, license, and README documentation.
