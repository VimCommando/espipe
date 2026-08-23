# Elasticsearch document pipe (espipe)

The goal of `espipe` is to be a minimalist command-line utility to bulk ingest documents from a file or I/O stream into an Elasticsearch cluster. There is no user-configured enrichment or transformation—and no complication. Supported local document formats are converted to Markdown during import.

Have you ever had thousands of sample documents in an `.ndjson` or `.csv` file, and you just want to load them all into a local insecure Elasticsearch cluster?

```bash
espipe docs.ndjson http://localhost:9200/new_index
```

And you're done.

Add a `my-cluster` host entry with API keys to the `~/.espipe/hosts.yml` and you can reference the host by name:

```bash
espipe docs.ndjson my-cluster:/new_index
```

## Description

Being multi-threaded and unthrottled, `espipe` is capable of fully saturating the CPU of the sending host and can potentially overwhelm the target cluster, so use with caution. It will gracefully handle backpressure and `http 429` responses to ensure at-least-once delivery.

Documents are batched into `_bulk` requests of 5,000 documents and sent with the `index` action by default. Use `--action` to select `create`, `index`, `update`, or `upsert`. Multi-source file-document inputs receive deterministic IDs based on their bundle and working-directory-relative path by default; single-source inputs require `--generate-id=true` to generate IDs. Use `--generate-id=false` to let Elasticsearch assign IDs for `create` and `index`, or to require explicit IDs for `update` and `upsert`. Use `--batch-size` and `--max-requests` to tune bulk request size and concurrency at runtime.

## Installation

Install the published crate with Cargo:

```bash
cargo install espipe
```

Run the published container image:

```bash
docker run --rm vimcommando/espipe --help
```

To build from source instead:

```bash
git clone https://github.com/VimCommando/espipe
cd espipe
cargo install --path .
```

To build the container image from source:

```bash
docker build -f docker/Dockerfile -t espipe:local .
```

To build and publish a multi-platform Docker Hub image:

```bash
./bin/buildx.sh
```

## What It Does

`espipe` reads records from:

- `.ndjson` files
- `.ndjson.gz` files
- `.json` files
- JSON arrays or objects selected with `--split`
- `.csv` files
- `.csv.gz` files
- local Markdown and text files
- local PDF, Word, PowerPoint, Excel, OpenDocument, RTF, and EPUB files
- local recursive glob patterns for file documents
- `stdin` as NDJSON

It writes records to:

- Elasticsearch `_bulk`
- a local `.ndjson` or `.ndjson.gz` file
- `stdout`

When writing to Elasticsearch, `espipe` batches documents into groups of 5,000 records by default, enables request body gzip compression by default, and sends multiple bulk requests concurrently. Use `--batch-size` to change the number of documents per bulk request and `--max-requests` to change the number of in-flight bulk requests. File gzip compression is selected only for supported `.csv.gz`, `.ndjson.gz`, and output `.ndjson.gz` suffixes, and is separate from Elasticsearch request body compression.

## CLI Reference

```bash
Usage: espipe [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...  Input URI(s) followed by the output URI

Options:
  -k, --insecure                     Ignore certificate validation
  -a, --apikey <APIKEY>              Apikey to authenticate via http header
  -u, --username <USERNAME>          Username for basic authentication
  -p, --password <PASSWORD>          Password for basic authentication
  -q, --quiet                        Quiet mode, don't print runtime summary
  -z, --uncompressed                 Disable request body gzip compression
      --content <CONTENT>            Content subfield name for file imports [default: body]
      --split <JSON_POINTER>         JSON Pointer selecting an array or object to split
      --action <ACTION>              Bulk action for Elasticsearch outputs [default: index] [possible values: create, index, update, upsert]
      --generate-id <GENERATE_ID>    Generate deterministic IDs for local files (default: multi-source only)
      --symlinks <SYMLINKS>          Multi-source symlink policy [default: skip] [possible values: follow, fail, skip]
      --hidden <HIDDEN>              Multi-source hidden-path policy [default: skip] [possible values: include, fail, skip]
      --batch-size <BATCH_SIZE>      Documents per Elasticsearch bulk request [default: 5000]
      --max-requests <MAX_REQUESTS>  Maximum concurrent Elasticsearch bulk requests [default: 16]
  -h, --help                         Print help
```

## Input And Output

All positional arguments are parsed as URI-like strings; one or more input URIs
are followed by the output URI.

### Supported input forms

- `-`
  Reads NDJSON from `stdin`.
- `path/to/file.ndjson`
  Reads NDJSON from a local file.
- `path/to/file.ndjson.gz`
  Reads gzip-compressed NDJSON from a local file.
- `path/to/file.json`
  Reads line-delimited JSON from a local file.
- `path/to/file.csv`
  Reads CSV from a local file.
- `path/to/file.csv.gz`
  Reads gzip-compressed CSV from a local file.
- `file:///absolute/path/to/file.ndjson`
  Reads NDJSON from a `file://` URI.
- `file:///absolute/path/to/file.ndjson.gz`
  Reads gzip-compressed NDJSON from a `file://` URI.
- `file:///absolute/path/to/file.json`
  Reads line-delimited JSON from a `file://` URI.
- `file:///absolute/path/to/file.csv`
  Reads CSV from a `file://` URI.
- `file:///absolute/path/to/file.csv.gz`
  Reads gzip-compressed CSV from a `file://` URI.
- `path/to/file.pdf`
  Converts a local PDF to Markdown and imports it as one file document.
- `path/to/file.docx`
  Converts a local Word document to Markdown and imports it as one file document.
- `'docs/**/*.pdf'`
  Recursively finds local PDFs and converts each one to a file document.
- `path/to/file.pdf path/to/file.xlsx output.ndjson`
  Imports multiple local file inputs in deterministic path order.

HTTP and HTTPS input URIs are supported for unauthenticated remote `.csv`, `.ndjson`, and `.json` sources. URLs without a supported file extension can still be accepted when the response `Content-Type` maps to CSV or NDJSON-oriented JSON input.

### AnyDoc local documents

Local files with these extensions are converted to GitHub-Flavored Markdown through anydoc before entering the existing file-document pipeline:

`.doc`, `.docx`, `.docm`, `.odt`, `.pdf`, `.ppt`, `.pps`, `.pot`, `.pptx`, `.pptm`, `.ppsx`, `.ppsm`, `.rtf`, `.epub`, `.xls`, `.xlsx`, `.xlsm`, `.xlsb`, `.ods`, and `.odp`.

Converted content is stored in `content.body` by default. Use `--content markdown` to store it in `content.markdown`. Every local file-document input adds an `origin` object with `scheme: file`, a working-directory-relative `path`, and `filename`; root-level files use `./` as the path. Remote CSV, NDJSON, and Toon inputs preserve the same components from their source URI. Anydoc conversion remains local-only. Per-file read or conversion errors in multi-file or glob imports, including globs that resolve to one file, are logged as warnings and skipped so later files continue. Scanned or image-only PDFs require OCR outside espipe and are skipped with a warning when they occur in a multi-file or glob import.

### Supported output forms

- `-`
  Writes raw JSON lines to `stdout`.
- `path/to/output.ndjson`
  Writes raw JSON lines to a local file, truncating any existing file.
- `path/to/output.ndjson.gz`
  Writes gzip-compressed raw JSON lines to a local file, truncating any existing file.
- `file:///absolute/path/to/output.ndjson`
  Writes raw JSON lines to a `file://` URI target.
- `file:///absolute/path/to/output.ndjson.gz`
  Writes gzip-compressed raw JSON lines to a `file://` URI target.
- `http://host:9200/index-name`
  Sends documents to Elasticsearch using the `_bulk` API.
- `https://host:9200/index-name`
  Sends documents to Elasticsearch over TLS.
- `known-host:index-name`
  Resolves `known-host` from a local hosts file and sends to the named index.

When writing to Elasticsearch, the output path must include an index name.

Remote `.json` inputs are treated as NDJSON. If the downloaded JSON payload does not match the required NDJSON shape, `espipe` exits with: `JSON payload does not look like required NDJSON input format.`

Passing `--split <JSON_POINTER>` instead treats each local input file as one JSON document and streams the children of the selected array or object. Split mode works with local paths, `file://` URIs, stdin, and HTTP/HTTPS JSON inputs. For multiple local files, the split is applied independently to each file.

## Data Format Rules

### NDJSON input

Each line must be valid line-delimited JSON. For pass-through JSON inputs, `espipe` expects the first non-whitespace character on each line to be `{`.

### Split JSON input

Use `--split /` to split a root JSON array or object. Use a JSON Pointer to drop wrappers and select a nested collection; for example, `--split /hits` and `--split /hits/` both select `hits`. One trailing slash is optional. Final empty-name members are not addressable, so paths with two trailing slashes such as `/hits//` are rejected. Pointer tokens use JSON Pointer escaping: `~1` represents `/` and `~0` represents `~`. Numeric tokens traverse intermediate arrays by zero-based index.

Each selected array element is emitted as one JSON object. Each selected object value is emitted as one JSON object with its property name added as a string `id` field. For eligible local files, the split key or array position also provides the transport discriminator for a generated Elasticsearch ID. Object values that already contain `id`, non-object children, missing paths, and selected scalar or null values are errors.

Split parsing is incremental and applies bounded backpressure through the existing output pipeline. Selected children are transformed in parallel batches using the machine's available CPU parallelism. Completed batches are forwarded immediately, so split mode does not guarantee source order for either arrays or objects. Include a sortable field in the source documents if downstream order matters.

The complete input, wrapper, and selected collection are never materialized, but bounded batches and their individual documents are. JSON parsing is still streaming rather than transactional, so an error late in the input does not roll back documents already sent; documents in concurrently running batches may already have reached the output.

### CSV input

The first row must be a header row. Each subsequent row is converted into a JSON object using the CSV headers as field names.

CSV values are emitted as JSON strings. `espipe` does not infer numeric, boolean, or date types from CSV input.

### Local file inputs

Markdown, text, YAML, structured JSON/NDJSON, CSV, Toon, and anydoc-converted files are emitted as JSON documents. Markdown frontmatter remains available under `content.*`; duplicate frontmatter keys warn and use the last value, while other invalid frontmatter remains fatal. Converted non-text files expose their generated Markdown under the configured content field. Existing file discovery supports shell-expanded paths, multiple local input positionals, and quoted recursive glob patterns.

Every local file document includes `origin.scheme: "file"`, a working-directory-relative `origin.path`, and `origin.filename`. Multi-source discovery skips symlinks and hidden paths by default. Use `--symlinks=follow|fail` to follow or reject symlinks, and `--hidden=include|fail` to include or reject hidden paths. `--symlinks=follow` preserves the supplied symlink path in `origin` and generated identity, including when its target is outside the working directory. Direct single-source inputs may reference external, hidden, or symlinked files without discovery-policy opt-ins.

Generated IDs are enabled by default for multi-source local inputs. Single-source inputs do not receive generated IDs unless `--generate-id=true` is passed. `--generate-id=false` disables generated IDs in either mode. Generated IDs are stable 22-character URL-safe Base64 values derived from the first 128 bits of a SHA-256 digest over the bundle identifier, relative source path, and a per-document discriminator; they do not depend on file contents or timestamps.

### Bulk actions

`espipe` supports four Elasticsearch bulk actions:

- `create`
  Sends each document as a `create` operation.
- `index`
  Sends each document as an `index` operation.
- `update`
  Sends each document as an `update` operation with a `{ "doc": ... }` payload.
- `upsert`
  Sends each document as an `update` operation with a `{ "doc": ..., "doc_as_upsert": true }` payload.

For `--action update` and `--action upsert`, every input document must:

- be a JSON object
- have an explicit string `_id`, or be a local file document with generated IDs enabled

For all Elasticsearch actions, a top-level string `_id` is used as the transport ID and removed from the source document. A tracked Git repository uses its repository directory name as the bundle identifier; otherwise the parent directory of the working path is used. Non-file inputs never receive generated IDs. Elasticsearch owns the operational behavior of each bulk action; espipe only constructs the request payload and interprets the response.

### Bulk tuning

For Elasticsearch targets:

- `--batch-size`
  Sets the number of documents included in each `_bulk` request.
- `--max-requests`
  Sets the maximum number of concurrent in-flight bulk requests.

The internal channel capacity always matches `--batch-size`.

## Output Behavior

### Elasticsearch output

For Elasticsearch targets, `espipe`:

- batches documents into 5,000-document `_bulk` requests by default
- keeps up to 16 bulk requests in flight by default
- enables gzip request body compression by default
- retries `429 Too Many Requests` responses with exponential backoff
- logs bulk-item error counts when Elasticsearch reports partial failures

`400 Bad Request` bulk responses are logged and counted as zero successful documents for that batch.

For local file imports, the completion summary reports all discovered files separately from documents sent and documents evaluated/read. Skipped files contribute to the file count but do not contribute documents. For example: `From 6,246 files, piped 5,850 of 5,850 docs ...`.

### File and stdout output

For file and `stdout` targets, `espipe` writes one raw JSON document per line. It does not emit Elasticsearch bulk action metadata lines for these outputs.

## Authentication And Known Hosts

Authentication flags apply only to direct `http://` and `https://` Elasticsearch outputs:

- `--apikey`
- `--username`
- `--password`
- `--insecure`

Known hosts are loaded from:

- `$ESPIPE_HOSTS`, if set
- otherwise `~/.espipe/hosts.yml`

Example:

```yaml
localhost:
  auth: None
  url: http://localhost:9200/

secure-cluster:
  auth: Basic
  url: https://example.com:9200/
  username: elastic
  password: changeme
  insecure: false

ess-cluster:
  auth: ApiKey
  url: https://cluster.example.com/
  apikey: "base64-encoded-api-key"
```

Usage:

```bash
espipe docs.ndjson localhost:my-index
espipe docs.ndjson secure-cluster:my-index
espipe docs.ndjson ess-cluster:my-index
```

For known-host outputs, authentication and TLS settings come from the host entry. CLI auth flags are not applied on top of the known-host configuration.

### Elastic CLI contexts

When `espipe` runs as an [Elastic CLI extension](https://github.com/elastic/cli), it uses the active Elasticsearch context passed by the CLI:

- `ELASTIC_ES_URL` supplies the Elasticsearch base URL.
- `ELASTIC_ES_API_KEY` supplies API-key authentication when no `--apikey`, `--username`, or `--password` option is provided.

Use `elasticsearch:/index` or `es:/index` as the final output argument to append an index to `ELASTIC_ES_URL`. These context schemes take precedence over same-named known-host entries. Other output forms, including bare local file paths, explicit `http://` or `https://` URLs, known hosts, `file://` outputs, and `-`, keep their existing behavior.

For example, these environment variables let an Elastic CLI extension ingest into the active context without exposing its credentials in the command line:

```bash
espipe docs.ndjson es:/my-index
```

## Examples

### Ingest NDJSON into a local Elasticsearch index

```bash
espipe docs.ndjson http://localhost:9200/my-index
```

### Ingest CSV into Elasticsearch

```bash
espipe users.csv http://localhost:9200/users
```

### Split a root JSON object into documents

```bash
espipe games.json http://localhost:9200/games --split /
```

### Split a wrapped JSON array into documents

```bash
espipe response.json output.ndjson --split /hits/
```

### Read NDJSON from stdin

```bash
cat docs.ndjson | espipe - http://localhost:9200/my-index
```

### Write normalized output to a file

```bash
espipe users.csv output.ndjson
```

### Ingest local PDFs recursively

```bash
espipe '**/*.pdf' http://localhost:9200/documents
```

### Ingest multiple local document formats

```bash
espipe '**/*.pdf' '**/*.docx' '**/*.xlsx' output.ndjson
```

### Read and write gzip-compressed files

```bash
espipe users.csv.gz output.ndjson.gz
espipe docs.ndjson.gz http://localhost:9200/my-index
```

### Use Elasticsearch basic authentication

```bash
espipe docs.ndjson https://example.com:9200/my-index \
  --username elastic \
  --password changeme
```

### Use an API key

```bash
espipe docs.ndjson https://example.com:9200/my-index \
  --apikey "base64-encoded-api-key"
```

### Use the active Elastic CLI context

```bash
elastic espipe docs.ndjson es:/my-index
```

### Disable gzip request body compression

```bash
espipe docs.ndjson http://localhost:9200/my-index --uncompressed
```

### Use a smaller bulk size with lower concurrency

```bash
espipe docs.ndjson http://localhost:9200/my-index \
  --batch-size 1000 \
  --max-requests 4
```

### Update existing documents by `_id`

Input:

```ndjson
{"_id":"1","message":"hello"}
{"_id":"2","message":"world"}
```

Command:

```bash
espipe docs.ndjson http://localhost:9200/my-index --action update
```

## Error Handling And Exit Behavior

`espipe` is optimized for straightforward ingestion, not for rich machine-readable error reporting.

Current behavior:

- invalid CLI argument combinations are rejected by `clap`
- invalid authentication combinations fail at startup
- invalid input or output targets fail at startup
- Elasticsearch transport failures during send or close terminate the process
- `429` bulk responses are retried automatically
- bulk item failures are logged, but successful items in the same batch are still counted

One current limitation is that input parsing errors and end-of-input are handled through the same loop boundary. In practice, malformed NDJSON or CSV input may stop ingestion early without a dedicated non-zero parsing exit code.

## Performance Notes

`espipe` is intentionally aggressive enough to saturate a local or small remote cluster.

Current bulk worker settings:

- batch size: 5,000 documents
- channel capacity: 5,000 documents
- max in-flight bulk requests: 16
- Tokio worker threads: 3

This is fast for local ingestion and test data loading, but it can overwhelm smaller clusters or shared environments.

## Troubleshooting

Set `LOG_LEVEL` to inspect request and ingestion behavior:

```bash
LOG_LEVEL=debug espipe docs.ndjson http://localhost:9200/my-index
```

Useful checks:

- verify the target index name is present in the output URI
- verify CSV files have a header row
- verify NDJSON files contain one complete JSON object per line
- verify update/upsert inputs include string `_id` values or have generated IDs enabled for eligible local file documents
- verify known-host entries live in `~/.espipe/hosts.yml` or `$ESPIPE_HOSTS`

## Scope

`espipe` is a binary crate. It does not publish or support a public Rust library interface.
