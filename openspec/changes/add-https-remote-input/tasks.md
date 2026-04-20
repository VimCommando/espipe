## 1. Input Fetching

- [ ] 1.1 Add a direct `reqwest` dependency and implement a startup-time HTTPS fetch helper that advertises supported CSV and NDJSON-oriented JSON media types.
- [ ] 1.2 Extend `Input::try_from` to recognize `https://` inputs, determine the input mode from URL extension or response `Content-Type`, download supported responses to temp files, and route them through the existing file-backed input variants.

## 2. Parsing And Validation

- [ ] 2.1 Reuse the existing CSV parsing path for fetched `.csv` content and the existing line-oriented JSON path for fetched `.ndjson` and `.json` content.
- [ ] 2.2 Return explicit startup errors for unsupported schemes, unrecognized remote formats, non-success HTTP statuses, transport failures, and `.json` payloads that do not match the required NDJSON shape.

## 3. Verification

- [ ] 3.1 Add tests for supported HTTPS remote input handling and failure cases using a local test server or equivalent HTTP mocking, including extensionless URLs resolved from response metadata.
- [ ] 3.2 Update user-facing documentation to replace the current “not implemented” note for supported HTTPS inputs and describe the `.json` NDJSON-only behavior and failure message.
