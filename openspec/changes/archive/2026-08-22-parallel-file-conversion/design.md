## Context

Multi-source local files currently share one synchronous `FileDocuments` iterator. Each call converts one complete source before the next path starts, and the async output handoff occurs only after conversion. Stable generated IDs depend on source paths and per-source document indexes rather than cross-source output order.

The CLI currently resolves `--batch-size` to 5,000 during argument parsing. Source cardinality becomes authoritative only after local path discovery and filtering.

## Goals / Non-Goals

**Goals:**

- Keep several CPU-bound file conversions active on machines with available cores.
- Bound active work and completed results.
- Emit completed sources without waiting for slower earlier paths while preserving existing file-level diagnostics.
- Select the implicit Elasticsearch batch size after input discovery.

**Non-Goals:**

- Parallelize records within one streaming file.
- Add a user-facing conversion-worker option in this change.
- Add OCR or change anydoc extraction behavior.
- Guarantee linear speedup across storage devices or document formats.

## Decisions

### Use a dedicated standard-library worker pool

Multi-source `FileDocuments` input will own a fixed set of worker threads. Each worker receives one source job at a time and returns the path and conversion result. Blocking conversion will not occupy a Tokio runtime worker.

The pool size will be the smaller of the source count, available parallelism, and eight workers. Eight allows the reported 500 to 600 percent CPU target while limiting simultaneous whole-file reads and nested parser work. A dedicated pool also avoids Tokio's much larger general blocking-thread limit. Adding Rayon directly was considered, but it would add a dependency without improving the existing channel-based output handoff.

### Emit results in completion order

The coordinator initially gives one source to each worker. When a result arrives, it immediately schedules the next source on that worker and returns the completed result to the output pipeline. The result channel is bounded by the worker count, so active and completed work remains bounded without an ordered result map.

This removes `BTreeMap` operations and retained out-of-order documents. More importantly, a slow early PDF cannot stop other workers after a fixed look-ahead window. File and stdout consumers that need order can sort by `origin.path` and `origin.filename`.

### Keep identity and error decisions on the consumer

Workers only read and convert a source into raw documents. The consumer logs failed sources, updates evaluated-document counters, and derives generated IDs from the source path and per-file document index. Completion order therefore cannot affect IDs, warning policy, or summary counts.

### Resolve the implicit bulk size from constructed input

`--batch-size` will become optional in the parsed CLI model. After local discovery constructs `Input`, the program will select 500 when the input reports more than one local source and 5,000 otherwise. An explicit value bypasses this selection.

Constructing local input before Elasticsearch output is safe because path discovery does not convert or emit documents. Remote and single-file streams retain the 5,000 default. The Elasticsearch output configuration remains explicit after selection, so its channel capacity continues to equal the effective batch size.

## Risks / Trade-offs

- [Nested parser parallelism can oversubscribe cores] -> Cap outer conversion workers at eight and benchmark the representative PDF collection.
- [Concurrent whole-file conversion increases peak memory and disk traffic] -> Bound active jobs and completed results by the worker count.
- [File and stdout output order changes across runs] -> Preserve source identity in `origin` and document IDs so consumers can sort when needed.
- [Input construction now precedes Elasticsearch preflight for local sources] -> Keep construction limited to validation and discovery; no conversion or output starts before preflight succeeds.

## Migration Plan

The behavior changes automatically for multi-source local imports. Users who need the old request size can pass `--batch-size 5000`. Rollback consists of restoring serial `FileDocuments` iteration and the static 5,000 default; document formats and generated IDs remain compatible.

## Validation

The release build was measured on 2026-08-22 against the 6,246-file NASA STI abstracts collection:

```bash
cd /Users/reno/Development/elastic-notes/samples
LOG_LEVEL=error /usr/bin/time -p /Users/reno/Development/espipe/target/release/espipe \
  'nasa-sti-abstracts/**/*.pdf' /tmp/espipe-nasa-parallel.ndjson
```

The ordered worker-pool candidate emitted 5,850 of 5,850 eligible documents in 8.768 seconds. Process timing reported 9.43 seconds real, 24.46 seconds user, and 6.31 seconds system. After removing ordered result buffering, the same command emitted the same document count in 5.185 seconds, with 5.68 seconds real, 24.37 seconds user, and 6.49 seconds system. Completion-order emission reduced application elapsed time by 40.9% while process CPU time stayed nearly flat, confirming that ordered delivery caused head-of-line waiting rather than useful work.

The document counts match the user's 23.388-second localhost Elasticsearch run. The outputs differ, so these measurements isolate conversion and local serialization rather than claiming a strict end-to-end Elasticsearch speedup. Repeating the exact Elasticsearch command would mutate the existing `localhost:/nasa-sti-abstracts` index and was left for an explicitly authorized run.
