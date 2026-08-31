# ESPIPE

_Minimum_ configuration, **Maximum** ingest

You have an Elasticsearch cluster. Have you ever...
- had to write your own custom client, just to ingest your documents?
- wanted to load a `.csv` files over 100MB into Elasticsearch?
- needed a quick way to re-load the same `.ndjson` across environments?
- wished for an easy way to convert and index thousands of `.docx`, `.pdf` or other documents?

Your answer is `espipe`. Easily stream documents from files or standard input into Elasticsearch. With on-the-fly document-to-markdown conversion now powered by [`anydoc`](https://github.com/firecrawl/anydoc).

No authentication? Just run:

```bash
espipe docs.ndjson http://localhost:9200/new_index
```

And you're done. Yes, it is that simple!

## Installation

Install with homebrew:

```bash
brew install VimCommando/tools/espipe
```

Run the published container image:

```bash
docker run --rm vimcommando/espipe --help
```

Install the published release with Cargo (compliles from source):

```bash
cargo install espipe
```

Clone repo and build latest `main` branch:

```bash
git clone https://github.com/VimCommando/espipe
cargo install --path espipe/
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

## Authentication

The only required configuration (if you have security on your cluster) is authentication. Use the URI scheme to specify what authentication to use.

### .elasticrc

If you're using the [Elastic CLI](https://github.com/elastic/cli/)? Use any `.elasticrc.yml` in a dot-context format `.context.service:`:

```bash
# current context, just use `.es` or `.elasticsearch`
espipe docs.ndjson .elasticsearch:/new_index

# `.es` service for the 'prod' context
espipe docs.ndjson .prod.es:/new_index
```

### Environment variables

Just define environment variables (or a `.env` file), and use the `env:` scheme

```bash
ELASTIC_ES_URL=http://localhost:9200 \
ELASTIC_ES_API_KEY=1234ABCD.... \
espipe docs.ndjson env:/new_index
```

### Custom hosts file

Define `my-cluster` in the `~/.espipe/hosts.yml` file:

```bash
espipe docs.ndjson my-cluster:/new_index
```

See more in the [authentication](docs/authentication.md) docs.

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

- `-` - Reads JSON lines from `stdin`.
- `dir/file.ext` - Reads a supported local data file.
- `dir/file.ext.gz` - Stream data from supported compressed files.
- `'dir/**/*.ext'` Recursively finds and converts each one to a file document.
- `https://host.co/file.json` - Reads from a remote file or endpoint (supports `Content-Type` or file extension)

See more in the [input](docs/input.md) docs.

### Supported output forms

- `-` - Writes JSON lines to `stdout`.
- `dir/file.ndjson` - Writes JSON lines to a local file.
- `dir/output.ndjson.gz` - Writes gzip-compressed JSON lines to a local file.
- `https://host:9200/index-name` - Sends documents to Elasticsearch using the `_bulk` API.
- `known-host:/index-name` - Resolves `known-host` from a local hosts file and sends to the named index.
- `env:/index-name` - Writes to the cluster URL and API key from environment variables or `.env` file.
- `.es:/index-name` - Writes to the Elasticsearch service from the current Elastic CLI context.
- `.context.es:/index-name` - Writes to the Elasticsearch service from the named Elastic CLI context.

When writing to Elasticsearch, the output path must include an index name.

See more in the [output](docs/output.md) docs.

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

### Piped tail input

Progressively load an `.ndjson`, line-by-line as its written, piped from `tail -f`, to the current Elastic CLI context:

```bash
tail -n 0 -f docs.ndjson | espipe --batch-size 1 - .es:/new_index
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
- verify context outputs use `.es:/index` or `.context.es:/index`, and that the selected context defines an Elasticsearch service
- verify `ELASTIC_CLI_CONFIG_FILE` points to a readable JSON or YAML Elastic CLI config when overriding home discovery
