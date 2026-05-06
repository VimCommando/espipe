## 1. Dependency and Fixtures

- [x] 1.1 Add the Toon parser dependency from `https://github.com/VimCommando/toon-rust` to `Cargo.toml` and update `Cargo.lock`.
- [x] 1.2 Add `.toon` test fixtures for single-document, multi-document, malformed, and non-object Toon input.
- [x] 1.3 Ensure package include metadata covers any checked-in `.toon` fixtures needed by tests.

## 2. Input Detection

- [x] 2.1 Add `Toon` to the internal input kind enum and map `.toon` paths to that kind.
- [x] 2.2 Update local file opening so `.toon` inputs use the Toon reader instead of whole-file document import.
- [x] 2.3 Update remote HTTPS detection to accept `.toon` paths and Toon response content types such as `application/toon` and `text/toon`.
- [x] 2.4 Update the remote `Accept` header to advertise Toon content alongside CSV and JSON-oriented formats.

## 3. Toon Reader

- [x] 3.1 Implement a streaming Toon reader variant for `Input` that reads `---`-separated document chunks and decodes each chunk with the forked parser.
- [x] 3.2 Convert each decoded Toon object to an owned `Box<RawValue>` before returning it from `read_next`.
- [x] 3.3 Reject decoded Toon arrays, scalars, and null values before they reach outputs.
- [x] 3.4 Surface Toon parse failures with diagnostics that include the input and document position when the parser exposes that information.

## 4. Remote Integration

- [x] 4.1 Route fetched remote `.toon` content through the same Toon reader used by local `.toon` files.
- [x] 4.2 Preserve existing non-success fetch and HTTPS-only behavior for Toon remote inputs.
- [x] 4.3 Add remote tests for `.toon` extension detection and Toon content-type fallback.

## 5. Verification

- [x] 5.1 Add unit tests for `.toon` input kind detection and unsupported compressed Toon behavior.
- [x] 5.2 Add input tests showing local Toon documents emit JSON object `RawValue` documents in input order.
- [x] 5.3 Add failure tests for malformed Toon and non-object Toon documents.
- [x] 5.4 Run `cargo test` and record any dependency/API caveats found while integrating the fork.
