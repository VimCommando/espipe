## 1. Parallel file conversion

- [x] 1.1 Add a bounded multi-source file conversion worker pool with machine-aware worker sizing and bounded scheduling.
- [x] 1.2 Integrate worker results with generated IDs, evaluated-document counts, and warn-and-skip error handling.
- [x] 1.3 Add tests proving conversions overlap while generated IDs remain deterministic.

## 2. Source-aware bulk defaults

- [x] 2.1 Defer implicit batch-size selection until input source cardinality is known.
- [x] 2.2 Default multi-source local input to 500 documents and retain 5,000 for single-file streaming and other inputs.
- [x] 2.3 Add CLI and configuration tests for implicit and explicit batch sizes.

## 3. Documentation and verification

- [x] 3.1 Update README and changelog text for parallel conversion and source-aware bulk defaults.
- [x] 3.2 Run formatting, static checks, targeted tests, and the full test suite.
- [x] 3.3 Benchmark the representative NASA PDF collection or record any environment blocker and the reproducible command.

## 4. Completion-order output

- [x] 4.1 Remove ordered result buffering and emit multi-source documents as conversions finish.
- [x] 4.2 Change the file-import summary to `Piped X of Y docs from Z files ...`.
- [x] 4.3 Update ordering tests, documentation, and changelog wording.
- [x] 4.4 Run validation and repeat the NASA benchmark to measure ordering overhead.

## 5. Verification fixes

- [x] 5.1 Correct duplicate-source semantics and specify the local import completion summary.
- [x] 5.2 Add a worker-level test for a failed conversion while another conversion is active.
- [x] 5.3 Run the full test suite and strict OpenSpec validation.
