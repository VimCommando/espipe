## Why

Large CSV and NDJSON transfers are common for `espipe`, and requiring users to decompress input files or post-process output files adds disk usage, time, and extra shell steps. Native gzip support lets `espipe` read and write compressed bulk data while preserving the existing file-oriented workflow.

## What Changes

- Accept local gzip-compressed NDJSON inputs with `.ndjson.gz`.
- Accept local gzip-compressed CSV inputs with `.csv.gz`.
- Write gzip-compressed NDJSON file output when the output path ends in `.ndjson.gz`.
- Keep uncompressed `.ndjson` and `.csv` behavior unchanged.
- Reject unsupported gzip suffixes clearly instead of attempting to infer arbitrary compressed formats.

## Capabilities

### New Capabilities
- `gzip-compression`: Defines gzip-compressed file input and output behavior for supported CSV and NDJSON formats.

### Modified Capabilities

## Impact

- Input format detection must recognize compound `.csv.gz` and `.ndjson.gz` suffixes.
- File readers need gzip decompression before the existing CSV and NDJSON parsers consume records.
- File output needs gzip compression when writing `.ndjson.gz`.
- Tests should cover compressed CSV input, compressed NDJSON input, compressed NDJSON file output, unsupported gzip suffixes, and unchanged uncompressed behavior.
- A gzip dependency may be added if the current dependency set does not already provide streaming gzip encode/decode support.
