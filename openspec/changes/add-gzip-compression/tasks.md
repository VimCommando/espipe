## 1. Format Detection

- [ ] 1.1 Add path helpers that recognize `.csv.gz`, `.ndjson.gz`, and `.ndjson` compound or simple suffixes without regressing existing extension detection.
- [ ] 1.2 Update local input kind detection so `.csv.gz` maps to CSV input and `.ndjson.gz` maps to NDJSON input.
- [ ] 1.3 Update multi-local-input output validation so `.ndjson.gz` is accepted anywhere `.ndjson` file output is accepted.
- [ ] 1.4 Add clear rejection paths for unsupported compressed input and output suffixes such as `.json.gz` and `.csv.gz` output.

## 2. Gzip Input

- [ ] 2.1 Add a streaming gzip decode dependency or reuse an existing dependency if available.
- [ ] 2.2 Wrap `.csv.gz` file readers in a gzip decoder before constructing the existing CSV reader.
- [ ] 2.3 Wrap `.ndjson.gz` file readers in a gzip decoder before constructing the existing NDJSON buffered reader.
- [ ] 2.4 Preserve existing uncompressed `.csv`, `.ndjson`, `.json`, stdin, and file-document import behavior.

## 3. Gzip Output

- [ ] 3.1 Generalize `FileOutput` so it can write through either a plain file writer or a gzip encoder while preserving the existing `Sender` API.
- [ ] 3.2 Select gzip-compressed output when the file output path ends in `.ndjson.gz`.
- [ ] 3.3 Ensure `Output::close` finalizes compressed output so the produced `.ndjson.gz` file is readable by standard gzip decoders.
- [ ] 3.4 Preserve existing stdout, Elasticsearch, and uncompressed file output behavior.

## 4. Tests

- [ ] 4.1 Add unit coverage for local input kind detection, including `.csv.gz`, `.ndjson.gz`, `.json.gz`, and existing uncompressed suffixes.
- [ ] 4.2 Add input tests proving `.csv.gz` and `.ndjson.gz` parse to the same documents as equivalent uncompressed files.
- [ ] 4.3 Add file output tests proving `.ndjson.gz` output decompresses to valid NDJSON.
- [ ] 4.4 Add CLI coverage for multi-file input writing to `.ndjson.gz`.
- [ ] 4.5 Add a checked-in `.ndjson.gz` fixture containing exactly 1,000 valid JSON object records.
- [ ] 4.6 Add an ignored localhost Elasticsearch integration test that ingests the 1,000-document `.ndjson.gz` fixture and verifies the refreshed index count is 1,000.
- [ ] 4.7 Add regression coverage that unsupported compressed suffixes fail clearly and that uncompressed behavior remains unchanged.

## 5. Documentation

- [ ] 5.1 Update README or CLI examples to show compressed CSV/NDJSON input and compressed NDJSON output.
- [ ] 5.2 Note that gzip file compression is suffix-based and separate from Elasticsearch request body compression.
