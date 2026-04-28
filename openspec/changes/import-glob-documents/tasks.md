## 1. CLI and Input Configuration

- [ ] 1.1 Add a `--content <field_name>` CLI argument in `src/main.rs` with default value `body`.
- [ ] 1.2 Validate that the configured content field name is non-empty before input construction.
- [ ] 1.3 Validate that the configured content field name does not contain `.` because it maps to `content.<field_name>`.
- [ ] 1.4 Change positional parsing so the final positional remains the output URI and every preceding positional is an input.
- [ ] 1.5 Pass all parsed input values and the configured content field name from the CLI into `Input::try_new` and test constructors.

## 2. Glob Discovery

- [ ] 2.1 Add file-document mode for multiple local input files and direct local plain-text files.
- [ ] 2.2 Add recursive glob pattern expansion for local inputs with glob metacharacters.
- [ ] 2.3 Combine concrete file inputs and glob matches, de-duplicate exact path matches, and filter to regular files.
- [ ] 2.4 Fail when file-document inputs resolve to no regular files.
- [ ] 2.5 Sort resolved file paths lexicographically before ingestion.
- [ ] 2.6 Preserve existing stdin, HTTPS, CSV, JSON, and NDJSON input behavior for non-file-document inputs.

## 3. File Document Construction

- [ ] 3.1 Add an `Input` variant that emits one raw JSON document per matched file.
- [ ] 3.2 Implement non-Markdown file imports by reading full file content into the configured `content.<field_name>` field.
- [ ] 3.3 Implement Markdown frontmatter detection using a leading `---` block.
- [ ] 3.4 Parse Markdown YAML frontmatter mappings into `content.<metadata_field>` JSON document fields.
- [ ] 3.5 Store Markdown content after frontmatter in the configured `content.<field_name>` field.
- [ ] 3.6 Add `file.path` and `file.name` fields only when input resolution produces more than one regular file.
- [ ] 3.7 Implement `.yml` and `.yaml` imports by converting YAML mapping roots into `content.<metadata_field>` fields.
- [ ] 3.8 Implement `.json` imports as whole-file JSON object parsing only.
- [ ] 3.9 Keep `.ndjson` and `.jsonl` imports as per-line JSON object parsing.
- [ ] 3.10 Reject JSON arrays, invalid YAML roots, invalid frontmatter, and content-field conflicts with file-specific stderr error messages.
- [ ] 3.11 Reject file-document inputs whose contents are not valid UTF-8 text with stderr diagnostics.

## 4. Tests and Verification

- [ ] 4.1 Add unit tests for shell-expanded file lists, direct Markdown/text files, recursive glob expansion, no-match failures, directory filtering, de-duplication, and deterministic ordering.
- [ ] 4.2 Add unit tests for default, custom, empty, and dotted content field names.
- [ ] 4.3 Add unit tests for conditional `file.path` and `file.name` metadata on multi-file imports only.
- [ ] 4.4 Add unit tests for Markdown frontmatter extraction into `content.*`, Markdown without frontmatter, non-mapping frontmatter rejection, and content-field conflict rejection.
- [ ] 4.5 Add unit tests for `.yml` and `.yaml` mapping imports into `content.*` and non-mapping root rejection.
- [ ] 4.6 Add unit tests for `.json` whole-object import, JSON array rejection, and non-object parse failure messages.
- [ ] 4.7 Add unit tests for `.ndjson` and `.jsonl` per-line object parsing and non-object line rejection.
- [ ] 4.8 Add unit tests for binary or invalid UTF-8 file rejection and stderr diagnostics.
- [ ] 4.9 Add regression tests proving existing CSV, NDJSON, JSON, stdin, and HTTPS input behavior still works.
- [ ] 4.10 Run `cargo test` and fix any failures.
