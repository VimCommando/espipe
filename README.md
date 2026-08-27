# Elasticsearch document pipe (espipe)

Have you ever had thousands of sample documents in an `.ndjson` or `.csv` file, and you just want to load them all into a local insecure Elasticsearch cluster?

You have an Elasticsearch cluster. Have you ever...
- had to write your own import client to import your documents?
- wanted to load a large `.csv` file into Elasticsearch?
- needed a quick way to re-load the same `.ndjson` across environments?
- wished for an easy way to convert and index thousands of `.docx`, `.pdf` or other documents?

Then `espipe` is your answer. Easily stream documents from files or standard input into Elasticsearch. Thanks to [`anydoc`](https://github.com/firecrawl/anydoc), it also converts supported documents (like `.docx`, `.pdf`, `.html` and more) to Markdown during import.

Just run:

```bash
espipe docs.ndjson http://localhost:9200/new_index
```

And you're done.

Define `my-cluster` in `~/.espipe/hosts.yml` to save the URL and credentials:

```bash
espipe docs.ndjson my-cluster:/new_index
```

## Installation

Install with homebrew:

```bash
brew install VimCommando/tools/espipe
```

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

## What it does

`espipe` reads records from:

- NDJSON, JSON, and CSV files, with gzip support for NDJSON and CSV
- JSON arrays or objects selected by `--split`
- local Markdown, text, YAML, JSONL, and Toon files
- local PDF, Word, PowerPoint, Excel, OpenDocument, RTF, and EPUB files
- local file lists and recursive subdirectory globs
- NDJSON from standard input

It writes records to:

- Elasticsearch `_bulk`
- a local `.ndjson` or `.ndjson.gz` file
- standard output

## CLI reference

```bash
espipe [OPTIONS] <INPUT>... <OUTPUT>
```

The main option groups are:

- input: `--content`, `--split`
- local discovery: `--generate-id`, `--symlinks`, `--hidden`
- Elasticsearch writes: `--action`, `--batch-size`, `--max-requests`, `--uncompressed`
- cluster setup: `--apikey`, `--username`, `--password`, `--insecure`, `--pipeline`, `--template`

Run `espipe --help` for all flags and accepted values.

## Input and output

Pass one or more inputs followed by one output. Local paths can also use `file://` URIs; the same format rules apply to both forms.

### Supported input forms

- `-`
  Reads NDJSON from `stdin`.
- `path/to/file.ndjson`, `path/to/file.json`, or `path/to/file.csv`
  Reads a supported local data file. Add `.gz` for compressed NDJSON or CSV.
- `path/to/file.pdf` or `path/to/file.docx`
  Converts a supported local document to Markdown.
- `'docs/**/*.pdf'`
  Recursively finds local PDFs and converts each one to a file document.
- `path/to/file.pdf path/to/file.xlsx output.ndjson`
  Imports multiple local file inputs and emits each source as its conversion finishes.

HTTP and HTTPS inputs support unauthenticated remote CSV, NDJSON, JSON, and Toon. If the URL has no recognized extension, `espipe` uses its `Content-Type`.

### AnyDoc local documents

Local files with these extensions are converted to GitHub-Flavored Markdown through anydoc before entering the existing file-document pipeline:

`.doc`, `.docx`, `.docm`, `.odt`, `.pdf`, `.ppt`, `.pps`, `.pot`, `.pptx`, `.pptm`, `.ppsx`, `.ppsm`, `.rtf`, `.epub`, `.xls`, `.xlsx`, `.xlsm`, `.xlsb`, `.ods`, and `.odp`.

Converted Markdown is stored in `content.body`; use `--content markdown` to change the field. Anydoc conversion is local only. Scanned PDFs require external OCR.

Multi-file and glob imports log per-file read or conversion failures and continue. Conversion uses up to eight workers and emits each source when it finishes, so cross-file order is unspecified. Generated IDs remain stable across runs, allowing for upsert operations.

### Supported output forms

- `-`
  Writes raw JSON lines to `stdout`.
- `path/to/output.ndjson`
  Writes raw JSON lines to a local file, truncating any existing file.
- `path/to/output.ndjson.gz`
  Writes gzip-compressed raw JSON lines to a local file, truncating any existing file.
- `http://host:9200/index-name`
  Sends documents to Elasticsearch using the `_bulk` API.
- `https://host:9200/index-name`
  Sends documents to Elasticsearch over TLS.
- `known-host:index-name`
  Resolves `known-host` from a local hosts file and sends to the named index.
- `env:/index-name`
  Reads the cluster URL and optional API key from environment variables or `.env`.

When writing to Elasticsearch, the output path must include an index name.

Remote `.json` inputs are treated as NDJSON. Use `--split <JSON_POINTER>` to treat JSON input as one document and stream a selected array or object's children. Split mode supports local input, standard input, and HTTP or HTTPS JSON. With multiple local files, it applies independently to each file.

## Data format rules

### NDJSON input

Each line must be valid line-delimited JSON. For pass-through JSON inputs, `espipe` expects the first non-whitespace character on each line to be `{`.

### Split JSON input

Use `--split /` for a root array or object, or a JSON Pointer such as `--split /hits` for a nested collection. One trailing slash is ignored. Pointer tokens use `~1` for `/` and `~0` for `~`; numeric tokens traverse arrays by zero-based index.

Each selected array element becomes one document. Each selected object value becomes one document with its key added as a string `id`. Existing `id` fields, non-object children, missing paths, and scalar or null selections are errors.

Split parsing is incremental and parallel. It does not preserve source order, and a late parse error does not roll back documents already sent.

### CSV input

The first row must be a header row. Each subsequent row is converted into a JSON object using the CSV headers as field names.

CSV values are emitted as JSON strings. `espipe` does not infer numeric, boolean, or date types from CSV input.

### Local file inputs

Markdown, text, YAML, JSON, NDJSON, JSONL, CSV, Toon, and anydoc-converted files become JSON documents. Markdown frontmatter is stored under `content.*`. Duplicate keys warn and use the last value; other invalid frontmatter is fatal.

Every local file document includes `origin.scheme: "file"`, a working-directory-relative `origin.path`, and `origin.filename`. Multi-source discovery skips symlinks and hidden paths by default. Use `--symlinks=follow|fail` and `--hidden=include|fail` to change those policies. Direct single-file input is not subject to discovery filtering.

Multi-source local inputs get deterministic IDs by default. Use `--generate-id=true` for a single source or `--generate-id=false` to disable them. IDs depend on the bundle, relative source path, and document position, not file contents or timestamps.

### Bulk actions

`--action` accepts `create`, `index`, `update`, or `upsert`. The default is `index`. Update wraps the source in `{ "doc": ... }`; upsert also adds `"doc_as_upsert": true`.

For `--action update` and `--action upsert`, every input document must:

- be a JSON object
- have an explicit string `_id`, or be a local file document with generated IDs enabled

For every action, a top-level string `_id` becomes the transport ID and is removed from the source. Non-file inputs never receive generated IDs.

### Bulk tuning

Multi-source local imports use 500-document bulk requests by default. Other inputs use 5,000. Override this with `--batch-size`; use `--max-requests` to change the default limit of 16 in-flight requests.

## Output behavior

### Elasticsearch output

Elasticsearch requests use gzip compression by default. `espipe` retries `429 Too Many Requests` responses with exponential backoff and logs item-level failures from partial bulk responses.

`400 Bad Request` bulk responses are logged and counted as zero successful documents for that batch.

Local import summaries separate discovered files from documents: `Piped 5,850 of 5,850 docs from 6,246 files ...`. Skipped files count as files, not documents.

### File and stdout output

For file and `stdout` targets, `espipe` writes one raw JSON document per line. It does not emit Elasticsearch bulk action metadata lines for these outputs.

## Authentication and known hosts

These flags apply to direct HTTP and HTTPS Elasticsearch outputs:

- `--apikey`
- `--username`
- `--password`
- `--insecure`

Known hosts load from `$ESPIPE_HOSTS` or, by default, `~/.espipe/hosts.yml`.

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

Known-host outputs use the authentication and TLS settings from their host entry.

### Environment targets

The `env:/index` output reads its connection settings from:

- `ELASTIC_ES_URL` supplies the Elasticsearch base URL.
- `ELASTIC_ES_API_KEY` supplies API-key authentication when no `--apikey`, `--username`, or `--password` option is provided.

Values already present in the process environment take precedence. For missing values, `espipe` searches the current directory and its parents for a `.env` file. The command fails if `ELASTIC_ES_URL` remains unset. This also works with environment variables supplied by an [Elastic CLI extension](https://github.com/elastic/cli).

```bash
espipe docs.ndjson env:/my-index
```

## Examples

### Ingest NDJSON into Elasticsearch

```bash
espipe docs.ndjson http://localhost:9200/my-index
```

### Split JSON into documents

```bash
espipe games.json http://localhost:9200/games --split /
espipe response.json output.ndjson --split /hits/
```

### Read NDJSON from stdin

```bash
cat docs.ndjson | espipe - http://localhost:9200/my-index
```

### Ingest local documents

```bash
espipe '**/*.pdf' http://localhost:9200/documents
espipe '**/*.pdf' '**/*.docx' '**/*.xlsx' output.ndjson
```

### Read and write gzip-compressed files

```bash
espipe users.csv.gz output.ndjson.gz
espipe docs.ndjson.gz http://localhost:9200/my-index
```

### Authenticate to Elasticsearch

```bash
espipe docs.ndjson https://example.com:9200/my-index \
  --username elastic \
  --password changeme
espipe docs.ndjson https://example.com:9200/my-index \
  --apikey "base64-encoded-api-key"
```

### Use the active Elastic CLI context

```bash
elastic espipe docs.ndjson env:/my-index
```

### Tune bulk requests

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

## Error handling and exit behavior

`espipe` writes diagnostics to standard error:

- invalid CLI argument combinations are rejected by `clap`
- invalid authentication combinations fail at startup
- invalid input or output targets fail at startup
- Elasticsearch transport failures during send or close terminate the process
- `429` bulk responses are retried automatically
- bulk item failures are logged, but successful items in the same batch are still counted

Malformed NDJSON or CSV may stop ingestion without a dedicated non-zero parsing exit code because parsing errors and end-of-input share the same loop boundary.

## Performance notes

Multi-source conversion uses up to eight workers, bounded by available parallelism and source count. Results are emitted as conversions finish, so one slow file does not hold back the rest. Bulk requests run concurrently and can saturate a local or small remote cluster; lower `--batch-size` or `--max-requests` when needed.

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
