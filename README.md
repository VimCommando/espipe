# Elasticsearch document pipe (espipe)

The goal of `espipe` is to be a minimalist command-line utility to bulk ingest documents from a file or I/O stream into an Elasticsearch cluster. No enrichment, no transformation, no complication.

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

Being multi-threaded and untrottled, `espipe` is capable of fully saturating the CPU of the sending host and can potentially overwhelm the target cluster, so use with caution. It will gracefully handle backpressure and `http 429` responses to ensure at-least-once delivery.

Documents are batched into `_bulk` requests of 5,000 documents and sent with the `create` action by default. Use `--action` to switch to `index` or `update` based on your needs. For `--action=update`, each source document must include an `_id` field for the update target. Use `--batch-size` and `--max-requests` to tune bulk request size and concurrency at runtime.

## Installation

Install the published crate with Cargo:

```bash
cargo install espipe
```

To build from source instead:

```bash
git clone https://github.com/VimCommando/espipe
cd espipe
cargo install --path .
```

## What It Does

`espipe` reads records from:

- `.ndjson` files
- `.csv` files
- `stdin` as NDJSON

It writes records to:

- Elasticsearch `_bulk`
- a local file
- `stdout`

When writing to Elasticsearch, `espipe` batches documents into groups of 5,000 records by default, enables request body gzip compression by default, and sends multiple bulk requests concurrently. Use `--batch-size` to change the number of documents per bulk request and `--max-requests` to change the number of in-flight bulk requests.

## CLI Reference

```bash
Usage: espipe [OPTIONS] <INPUT> <OUTPUT>

Arguments:
  <INPUT>   The input URI to read docs from
  <OUTPUT>  The output URI to send docs to

Options:
  -k, --insecure                     Ignore certificate validation
  -a, --apikey <APIKEY>              Apikey to authenticate via http header
  -u, --username <USERNAME>          Username for basic authentication
  -p, --password <PASSWORD>          Password for basic authentication
  -q, --quiet                        Quiet mode, don't print runtime summary
  -z, --uncompressed                 Disable request body gzip compression
      --action <ACTION>              Bulk action for Elasticsearch outputs [default: create] [possible values: create, index, update]
      --batch-size <BATCH_SIZE>      Documents per Elasticsearch bulk request [default: 5000]
      --max-requests <MAX_REQUESTS>  Maximum concurrent Elasticsearch bulk requests [default: 16]
  -h, --help                         Print help
```

## Input And Output

Both positional arguments are parsed as URI-like strings.

### Supported input forms

- `-`
  Reads NDJSON from `stdin`.
- `path/to/file.ndjson`
  Reads NDJSON from a local file.
- `path/to/file.csv`
  Reads CSV from a local file.
- `file:///absolute/path/to/file.ndjson`
  Reads NDJSON from a `file://` URI.
- `file:///absolute/path/to/file.csv`
  Reads CSV from a `file://` URI.

HTTPS input URIs are supported for unauthenticated remote `.csv`, `.ndjson`, and `.json` sources. URLs without a supported file extension can still be accepted when the response `Content-Type` maps to CSV or NDJSON-oriented JSON input.

### Supported output forms

- `-`
  Writes raw JSON lines to `stdout`.
- `path/to/output.ndjson`
  Writes raw JSON lines to a local file, truncating any existing file.
- `file:///absolute/path/to/output.ndjson`
  Writes raw JSON lines to a `file://` URI target.
- `http://host:9200/index-name`
  Sends documents to Elasticsearch using the `_bulk` API.
- `https://host:9200/index-name`
  Sends documents to Elasticsearch over TLS.
- `known-host:index-name`
  Resolves `known-host` from a local hosts file and sends to the named index.

When writing to Elasticsearch, the output path must include an index name.

Remote `.json` inputs are treated as NDJSON. If the downloaded JSON payload does not match the required NDJSON shape, `espipe` exits with: `JSON payload does not look like required NDJSON input format.`

## Data Format Rules

### NDJSON input

Each line must be a valid JSON object. `espipe` forwards the JSON document body directly without reformatting.

### CSV input

The first row must be a header row. Each subsequent row is converted into a JSON object using the CSV headers as field names.

CSV values are emitted as JSON strings. `espipe` does not infer numeric, boolean, or date types from CSV input.

### Bulk actions

`espipe` supports three Elasticsearch bulk actions:

- `create`
  Sends each document as a `create` operation.
- `index`
  Sends each document as an `index` operation.
- `update`
  Sends each document as an `update` operation with a `{ "doc": ... }` payload.

For `--action update`, every input document must:

- be a JSON object
- include an `_id` field
- have `_id` as a string

The `_id` field is removed from the document body and used as the update target.

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

## Examples

### Ingest NDJSON into a local Elasticsearch index

```bash
espipe docs.ndjson http://localhost:9200/my-index
```

### Ingest CSV into Elasticsearch

```bash
espipe users.csv http://localhost:9200/users
```

### Read NDJSON from stdin

```bash
cat docs.ndjson | espipe - http://localhost:9200/my-index
```

### Write normalized output to a file

```bash
espipe users.csv output.ndjson
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
- verify `--action update` inputs include string `_id` values
- verify known-host entries live in `~/.espipe/hosts.yml` or `$ESPIPE_HOSTS`

## Scope

`espipe` is a binary crate. It does not publish or support a public Rust library interface.
