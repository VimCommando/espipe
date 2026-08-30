mod client;
mod input;
mod json_split;
mod output;

use clap::Parser;
use client::Auth;
use fluent_uri::UriRef;
use input::{DiscoveryOptions, HiddenMode, Input, SymlinkMode};
use json_split::SplitPath;
use output::{BulkAction, ElasticsearchOutputConfig, Output, OutputPreflightConfig, OutputTarget};
use std::{env, path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(version)]
struct Cli {
    /// The input(s) to read docs from, followed by the output URI
    #[arg(
        help = "Input URI(s) followed by the output URI or .context.es:/index",
        required = true,
        num_args = 2..
    )]
    paths: Vec<String>,
    /// Content subfield name for file imports
    #[arg(
        help = "Content subfield name for file imports",
        long,
        default_value = "body"
    )]
    content: String,
    /// Split a JSON array or object selected by JSON Pointer
    #[arg(
        help = "JSON Pointer selecting an array or object to split",
        long,
        value_name = "JSON_POINTER"
    )]
    split: Option<String>,
    /// Accept invalid certificates
    #[arg(
        help = "Ignore certificate validation",
        long,
        short = 'k',
        default_value = "false"
    )]
    insecure: bool,
    /// ApiKey for authentication
    #[arg(help = "Apikey to authenticate via http header", long, short)]
    apikey: Option<String>,
    /// Username for authentication
    #[arg(
        help = "Username for basic authentication",
        long,
        short,
        conflicts_with = "apikey",
        requires = "password"
    )]
    username: Option<String>,
    /// Password for basic authentication
    #[arg(
        help = "Password for basic authentication",
        long,
        short,
        conflicts_with = "apikey",
        requires = "username"
    )]
    password: Option<String>,
    /// Quiet mode, don't print summary line
    #[arg(
        help = "Quiet mode, don't print runtime summary",
        long,
        short = 'q',
        default_value = "false"
    )]
    quiet: bool,
    /// Disable request body compression
    #[arg(
        help = "Disable request body gzip compression",
        long,
        short = 'z',
        default_value = "false"
    )]
    uncompressed: bool,
    /// Bulk action for Elasticsearch outputs
    #[arg(
        help = "Bulk action for Elasticsearch outputs",
        long,
        value_enum,
        default_value_t = BulkAction::Index
    )]
    action: BulkAction,
    /// Generate deterministic IDs for local file inputs; defaults by source cardinality
    #[arg(
        help = "Generate deterministic IDs for local file inputs (default: multi-source only)",
        long,
        action = clap::ArgAction::Set
    )]
    generate_id: Option<bool>,
    /// Multi-source symlink discovery policy
    #[arg(
        help = "Multi-source symlink policy",
        long,
        value_enum,
        default_value_t = SymlinkMode::Skip
    )]
    symlinks: SymlinkMode,
    /// Multi-source hidden-path discovery policy
    #[arg(
        help = "Multi-source hidden-path policy",
        long,
        value_enum,
        default_value_t = HiddenMode::Skip
    )]
    hidden: HiddenMode,
    /// Documents per Elasticsearch bulk request
    #[arg(
        help = "Documents per Elasticsearch bulk request (default: 500 for multi-source local input, 5000 otherwise)",
        long,
        value_parser = parse_nonzero_usize
    )]
    batch_size: Option<usize>,
    /// Maximum concurrent Elasticsearch bulk requests
    #[arg(
        help = "Maximum concurrent Elasticsearch bulk requests",
        long,
        default_value_t = ElasticsearchOutputConfig::DEFAULT_MAX_INFLIGHT_REQUESTS,
        value_parser = parse_nonzero_usize
    )]
    max_requests: usize,
    /// Elasticsearch ingest pipeline JSON or YAML file to install before bulk indexing
    #[arg(help = "Elasticsearch ingest pipeline JSON or YAML file", long)]
    pipeline: Option<PathBuf>,
    /// Elasticsearch ingest pipeline name override
    #[arg(help = "Elasticsearch ingest pipeline name", long)]
    pipeline_name: Option<String>,
    /// Composable index template file or bundled selector to install before ingestion
    #[arg(
        help = "Composable index template file or bundled selector such as _okf for Elasticsearch outputs; file extensions .json, .jsonc, .json5, .yml, and .yaml are detected, and other files are parsed as strict JSON",
        long
    )]
    template: Option<PathBuf>,
    /// Override the file-derived or bundled default template name
    #[arg(
        help = "Composable index template name override; bundled templates otherwise use their embedded default name",
        long
    )]
    template_name: Option<String>,
    /// Allow replacement or bundled index-pattern updates
    #[arg(
        help = "Allow replacement of a file template or index-pattern updates to a bundled template",
        long
    )]
    template_overwrite: Option<bool>,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    let start_time = std::time::Instant::now();
    let env = env_logger::Env::default().filter_or("LOG_LEVEL", "warn");
    env_logger::Builder::from_env(env)
        .format_timestamp_millis()
        .init();

    let args = Cli::parse();
    let Cli {
        mut paths,
        content,
        split,
        quiet,
        insecure,
        apikey,
        password,
        username,
        uncompressed,
        action,
        generate_id,
        symlinks,
        hidden,
        batch_size,
        max_requests,
        pipeline,
        pipeline_name,
        template,
        template_name,
        template_overwrite,
    } = args;
    let output = match OutputTarget::parse(paths.pop().expect("clap requires at least two paths")) {
        Ok(output) => output,
        Err(err) => return exit_with_error(err),
    };
    let environment_output = match Output::validate_environment_target(&output) {
        Ok(environment_output) => environment_output,
        Err(err) => return exit_with_error(err),
    };
    if environment_output && let Err(err) = load_dotenv() {
        return exit_with_error(err);
    }
    let inputs = match parse_input_uris(paths) {
        Ok(inputs) => inputs,
        Err(err) => return exit_with_error(err),
    };
    let split = match split {
        Some(path) => match SplitPath::parse(&path) {
            Ok(path) => Some(path),
            Err(err) => return exit_with_error(err),
        },
        None => None,
    };
    if let Err(err) = validate_multi_input_output(&inputs, &output) {
        return exit_with_error(err);
    }

    let apikey = resolve_api_key(
        apikey,
        username.as_deref(),
        password.as_deref(),
        environment_output.then(environment_api_key).flatten(),
    );
    let auth = match Auth::try_new(apikey, username, password) {
        Ok(auth) => auth,
        Err(err) => return exit_with_error(err),
    };
    let preflight = OutputPreflightConfig {
        pipeline,
        pipeline_name,
        template,
        template_name,
        template_overwrite,
    };
    if let Err(err) = preflight.validate() {
        return exit_with_error(err);
    }
    if let Err(err) = Output::validate_preflight_target(&output, &preflight) {
        return exit_with_error(err);
    }

    let discovery_options = DiscoveryOptions { symlinks, hidden };
    let bundled_template = preflight
        .template
        .as_ref()
        .and_then(|path| path.to_str())
        .is_some_and(|value| value.starts_with('_'));
    let multi_source_local = if bundled_template && batch_size.is_none() {
        match Input::discover_is_multi_source_local(&inputs, discovery_options) {
            Ok(multi_source_local) => multi_source_local,
            Err(err) => return exit_with_error(err),
        }
    } else {
        false
    };
    let discover_before_output =
        should_discover_input_before_output(batch_size, &inputs, bundled_template);
    let (mut input, mut output) = if discover_before_output {
        let input =
            match Input::try_new(inputs, content, split, generate_id, discovery_options).await {
                Ok(input) => input,
                Err(err) => return exit_with_error(err),
            };
        log::debug!("input: {input}");

        let batch_size = effective_batch_size(batch_size, input.is_multi_source_local());
        let elasticsearch_config =
            match ElasticsearchOutputConfig::try_new(batch_size, max_requests) {
                Ok(config) => config,
                Err(err) => return exit_with_error(err),
            };
        let output = match Output::try_new(
            insecure,
            auth,
            output,
            environment_output.then(environment_url).flatten(),
            action,
            !uncompressed,
            elasticsearch_config,
            preflight,
        )
        .await
        {
            Ok(output) => output,
            Err(err) => return exit_with_error(err),
        };
        log::debug!("output: {output}");
        (input, output)
    } else {
        let batch_size = effective_batch_size(batch_size, multi_source_local);
        let elasticsearch_config =
            match ElasticsearchOutputConfig::try_new(batch_size, max_requests) {
                Ok(config) => config,
                Err(err) => return exit_with_error(err),
            };
        let output = match Output::try_new(
            insecure,
            auth,
            output,
            environment_output.then(environment_url).flatten(),
            action,
            !uncompressed,
            elasticsearch_config,
            preflight,
        )
        .await
        {
            Ok(output) => output,
            Err(err) => return exit_with_error(err),
        };
        log::debug!("output: {output}");

        let input =
            match Input::try_new(inputs, content, split, generate_id, discovery_options).await {
                Ok(input) => input,
                Err(err) => return exit_with_error(err),
            };
        log::debug!("input: {input}");
        (input, output)
    };

    let mut input_line: usize = 0;
    let mut output_line: usize = 0;
    let output_name = output.to_string();
    let mut line_buffer = String::with_capacity(1024);
    loop {
        let line = match input.read_next(&mut line_buffer) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(err) => {
                if let Err(close_err) = output.close().await {
                    eprintln!("Could not close output after input error: {close_err}");
                }
                return exit_with_error(err);
            }
        };
        input_line += 1;
        match output.send(line).await {
            Ok(sent) => output_line += sent,
            Err(err) => return exit_with_error(err),
        }
        line_buffer.clear();
    }
    output_line += match output.close().await {
        Ok(sent) => sent,
        Err(err) => return exit_with_error(err),
    };
    if !quiet {
        let evaluated_line = input.evaluated_document_count(input_line);
        if let Some(file_count) = input.file_count() {
            let file_label = if file_count == 1 { "file" } else { "files" };
            println!(
                "Piped {} of {} docs from {} {file_label} to {output_name} in {:.3} seconds",
                comma_formatted(output_line),
                comma_formatted(evaluated_line),
                comma_formatted(file_count),
                start_time.elapsed().as_secs_f32()
            );
        } else {
            println!(
                "Piped {} of {} docs to {output_name} in {:.3} seconds",
                comma_formatted(output_line),
                comma_formatted(evaluated_line),
                start_time.elapsed().as_secs_f32()
            );
        }
    }
    ExitCode::SUCCESS
}

fn comma_formatted(number: usize) -> String {
    let string = number.to_string();
    let len = string.len();
    let mut result = String::with_capacity(len + len / 3);

    for (i, c) in string.chars().enumerate() {
        result.push(c);
        let pos = len - i - 1;
        if pos > 0 && pos % 3 == 0 {
            result.push(',');
        }
    }

    result
}

fn exit_with_error(err: eyre::Report) -> ExitCode {
    eprintln!("{err}");
    ExitCode::FAILURE
}

fn validate_multi_input_output(
    inputs: &[UriRef<String>],
    output: &OutputTarget,
) -> eyre::Result<()> {
    if inputs.len() <= 1 {
        return Ok(());
    }
    if !inputs.iter().all(is_local_file_input) {
        return Ok(());
    }

    if !output.is_file_output() {
        return Ok(());
    }

    if output.file_path().is_some_and(is_ndjson_file_output) {
        return Ok(());
    }

    Err(eyre::eyre!(
        "multiple file inputs require a file output path ending in .ndjson or .ndjson.gz"
    ))
}

fn parse_input_uris(inputs: Vec<String>) -> eyre::Result<Vec<UriRef<String>>> {
    inputs
        .into_iter()
        .map(|input| {
            UriRef::parse(input)
                .map_err(|(err, input)| eyre::eyre!("invalid input URI '{input}': {err}"))
        })
        .collect()
}

fn is_ndjson_file_output(path: &str) -> bool {
    let lower_path = path.to_ascii_lowercase();
    lower_path.ends_with(".ndjson") || lower_path.ends_with(".ndjson.gz")
}

fn is_local_file_input(input: &UriRef<String>) -> bool {
    matches!(
        input.scheme().map(|scheme| scheme.as_str()),
        Some("file") | None
    ) && input.path().as_str() != "-"
}

fn parse_nonzero_usize(value: &str) -> Result<usize, String> {
    let parsed = value.parse::<usize>().map_err(|err| err.to_string())?;
    if parsed == 0 {
        return Err("value must be at least 1".to_string());
    }
    Ok(parsed)
}

fn effective_batch_size(explicit: Option<usize>, multi_source_local: bool) -> usize {
    explicit.unwrap_or(if multi_source_local {
        ElasticsearchOutputConfig::MULTI_SOURCE_DEFAULT_BATCH_SIZE
    } else {
        ElasticsearchOutputConfig::DEFAULT_BATCH_SIZE
    })
}

fn should_discover_input_before_output(
    explicit_batch_size: Option<usize>,
    inputs: &[UriRef<String>],
    bundled_template: bool,
) -> bool {
    !bundled_template
        && (inputs.len() > 1
            || (explicit_batch_size.is_none() && inputs.iter().all(is_local_file_input)))
}

fn load_dotenv() -> eyre::Result<()> {
    match dotenvy::dotenv() {
        Ok(_) => Ok(()),
        Err(dotenvy::Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(eyre::eyre!("Could not read .env: {err}")),
    }
}

fn environment_url() -> Option<String> {
    env::var("ELASTIC_ES_URL").ok()
}

fn environment_api_key() -> Option<String> {
    env::var("ELASTIC_ES_API_KEY").ok()
}

fn resolve_api_key(
    apikey: Option<String>,
    username: Option<&str>,
    password: Option<&str>,
    environment_api_key: Option<String>,
) -> Option<String> {
    if apikey.is_some() || username.is_some() || password.is_some() {
        apikey
    } else {
        environment_api_key
    }
}

#[cfg(test)]
mod tests {
    use super::{effective_batch_size, resolve_api_key, should_discover_input_before_output};
    use crate::output::ElasticsearchOutputConfig;
    use fluent_uri::UriRef;

    #[test]
    fn environment_api_key_is_used_without_explicit_authentication() {
        assert_eq!(
            resolve_api_key(None, None, None, Some("context-key".to_string())),
            Some("context-key".to_string())
        );
    }

    #[test]
    fn explicit_authentication_takes_precedence_over_environment_api_key() {
        assert_eq!(
            resolve_api_key(
                Some("command-line-key".to_string()),
                None,
                None,
                Some("context-key".to_string()),
            ),
            Some("command-line-key".to_string())
        );
        assert_eq!(
            resolve_api_key(
                None,
                Some("elastic"),
                Some("password"),
                Some("context-key".to_string())
            ),
            None
        );
    }

    #[test]
    fn batch_size_default_depends_on_local_source_count() {
        assert_eq!(
            effective_batch_size(None, true),
            ElasticsearchOutputConfig::MULTI_SOURCE_DEFAULT_BATCH_SIZE
        );
        assert_eq!(
            effective_batch_size(None, false),
            ElasticsearchOutputConfig::DEFAULT_BATCH_SIZE
        );
    }

    #[test]
    fn explicit_batch_size_overrides_input_default() {
        assert_eq!(effective_batch_size(Some(750), true), 750);
        assert_eq!(effective_batch_size(Some(750), false), 750);
    }

    #[test]
    fn multi_input_validation_and_implicit_local_batch_sizes_precede_output() {
        let local = UriRef::parse("docs/**/*.pdf".to_string()).unwrap();
        let remote = UriRef::parse("https://example.com/docs.ndjson".to_string()).unwrap();
        let second_remote = UriRef::parse("https://example.com/more.ndjson".to_string()).unwrap();
        let stdin = UriRef::parse("-".to_string()).unwrap();

        assert!(should_discover_input_before_output(
            None,
            &[local.clone()],
            false
        ));
        assert!(!should_discover_input_before_output(
            Some(750),
            &[local.clone()],
            false
        ));
        assert!(!should_discover_input_before_output(None, &[local], true));
        assert!(!should_discover_input_before_output(
            None,
            &[remote.clone()],
            false
        ));
        assert!(should_discover_input_before_output(
            Some(750),
            &[remote, second_remote],
            false
        ));
        assert!(!should_discover_input_before_output(None, &[stdin], false));
    }
}
