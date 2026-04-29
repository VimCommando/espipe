## 1. CLI and Input Configuration

- [x] 1.1 Add a `--content <field_name>` CLI argument in `src/main.rs` with default value `body`.
- [x] 1.2 Validate that the configured content field name is non-empty before input construction.
- [x] 1.3 Validate that the configured content field name does not contain `.` because it maps to `content.<field_name>`.
- [x] 1.4 Change positional parsing so the final positional remains the output URI and every preceding positional is an input.
- [x] 1.5 Pass all parsed input values and the configured content field name from the CLI into `Input::try_new` and test constructors.

## 2. Glob Discovery

- [x] 2.1 Add file-document mode for multiple local input files and direct local plain-text files.
- [x] 2.2 Add recursive glob pattern expansion for local inputs with glob metacharacters.
- [x] 2.3 Combine concrete file inputs and glob matches, de-duplicate exact path matches, and filter to regular files.
- [x] 2.4 Fail when file-document inputs resolve to no regular files.
- [x] 2.5 Sort resolved file paths lexicographically before ingestion.
- [x] 2.6 Preserve existing stdin, HTTPS, CSV, JSON, and NDJSON input behavior for non-file-document inputs.

## 3. File Document Construction

- [x] 3.1 Add an `Input` variant that emits one raw JSON document per matched file.
- [x] 3.2 Implement non-Markdown file imports by reading full file content into the configured `content.<field_name>` field.
- [x] 3.3 Implement Markdown frontmatter detection using a leading `---` block.
- [x] 3.4 Parse Markdown YAML frontmatter mappings into `content.<metadata_field>` JSON document fields.
- [x] 3.5 Store Markdown content after frontmatter in the configured `content.<field_name>` field.
- [x] 3.6 Add `file.path` and `file.name` fields only when input resolution produces more than one regular file.
- [x] 3.7 Implement `.yml` and `.yaml` imports by converting YAML mapping roots into `content.<metadata_field>` fields.
- [x] 3.8 Implement `.json` imports as whole-file JSON object parsing only.
- [x] 3.9 Keep `.ndjson` and `.jsonl` imports as per-line JSON object parsing.
- [x] 3.10 Reject JSON arrays, invalid YAML roots, invalid frontmatter, and content-field conflicts with file-specific stderr error messages.
- [x] 3.11 Reject file-document inputs whose contents are not valid UTF-8 text with stderr diagnostics.

## 4. Tests and Verification

- [x] 4.1 Add unit tests for shell-expanded file lists, direct Markdown/text files, recursive glob expansion, no-match failures, directory filtering, de-duplication, and deterministic ordering.
- [x] 4.2 Add unit tests for default, custom, empty, and dotted content field names.
- [x] 4.3 Add unit tests for conditional `file.path` and `file.name` metadata on multi-file imports only.
- [x] 4.4 Add unit tests for Markdown frontmatter extraction into `content.*`, Markdown without frontmatter, non-mapping frontmatter rejection, and content-field conflict rejection.
- [x] 4.5 Add unit tests for `.yml` and `.yaml` mapping imports into `content.*` and non-mapping root rejection.
- [x] 4.6 Add unit tests for `.json` whole-object import, JSON array rejection, and non-object parse failure messages.
- [x] 4.7 Add unit tests for `.ndjson` and `.jsonl` per-line object parsing and non-object line rejection.
- [x] 4.8 Add unit tests for binary or invalid UTF-8 file rejection and stderr diagnostics.
- [x] 4.9 Add regression tests proving existing CSV, NDJSON, JSON, stdin, and HTTPS input behavior still works.
- [x] 4.10 Run `cargo test` and fix any failures.
