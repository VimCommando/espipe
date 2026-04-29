mod client;
mod input;
mod output;

use clap::Parser;
use client::Auth;
use fluent_uri::UriRef;
use input::Input;
use output::{BulkAction, ElasticsearchOutputConfig, Output, OutputPreflightConfig};
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
struct Cli {
    /// The input(s) to read docs from, followed by the output URI
    #[arg(
        help = "Input URI(s) followed by the output URI",
        required = true,
        num_args = 2..
    )]
    paths: Vec<UriRef<String>>,
    /// Content subfield name for file imports
    #[arg(
        help = "Content subfield name for file imports",
        long,
        default_value = "body"
    )]
    content: String,
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
        default_value_t = BulkAction::Create
    )]
    action: BulkAction,
    /// Documents per Elasticsearch bulk request
    #[arg(
        help = "Documents per Elasticsearch bulk request",
        long,
        default_value_t = ElasticsearchOutputConfig::DEFAULT_BATCH_SIZE,
        value_parser = parse_nonzero_usize
    )]
    batch_size: usize,
    /// Maximum concurrent Elasticsearch bulk requests
    #[arg(
        help = "Maximum concurrent Elasticsearch bulk requests",
        long,
        default_value_t = ElasticsearchOutputConfig::DEFAULT_MAX_INFLIGHT_REQUESTS,
        value_parser = parse_nonzero_usize
    )]
    max_requests: usize,
    /// Elasticsearch ingest pipeline JSON file to install before bulk indexing
    #[arg(help = "Elasticsearch ingest pipeline JSON file", long)]
    pipeline: Option<PathBuf>,
    /// Elasticsearch ingest pipeline name override
    #[arg(help = "Elasticsearch ingest pipeline name", long)]
    pipeline_name: Option<String>,
    /// Composable index template file to install before Elasticsearch bulk ingestion
    #[arg(
        help = "Composable index template file for Elasticsearch outputs",
        long
    )]
    template: Option<PathBuf>,
    /// Override the template name; defaults to the template file name without its final extension
    #[arg(help = "Composable index template name override", long)]
    template_name: Option<String>,
    /// Overwrite an existing composable index template
    #[arg(help = "Overwrite an existing composable index template", long)]
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
        quiet,
        insecure,
        apikey,
        password,
        username,
        uncompressed,
        action,
        batch_size,
        max_requests,
        pipeline,
        pipeline_name,
        template,
        template_name,
        template_overwrite,
    } = args;
    let output = paths.pop().expect("clap requires at least two paths");
    let inputs = paths;

    let auth = match Auth::try_new(apikey, username, password) {
        Ok(auth) => auth,
        Err(err) => return exit_with_error(err),
    };
    let elasticsearch_config = match ElasticsearchOutputConfig::try_new(batch_size, max_requests) {
        Ok(config) => config,
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

    let (mut input, mut output) = if preflight.has_elasticsearch_options() {
        let output = match Output::try_new(
            insecure,
            auth,
            output,
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

        let input = match Input::try_new(inputs, content).await {
            Ok(input) => input,
            Err(err) => return exit_with_error(err),
        };
        log::debug!("input: {input}");
        (input, output)
    } else {
        let input = match Input::try_new(inputs, content).await {
            Ok(input) => input,
            Err(err) => return exit_with_error(err),
        };
        log::debug!("input: {input}");

        let output = match Output::try_new(
            insecure,
            auth,
            output,
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
    };

    let mut input_line: usize = 0;
    let mut output_line: usize = 0;
    let output_name = output.to_string();
    let mut line_buffer = String::with_capacity(1024);
    while let Ok(line) = input.read_line(&mut line_buffer) {
        input_line += 1;
        output_line += match output.send(line).await {
            Ok(sent) => sent,
            Err(err) => return exit_with_error(err),
        };
        line_buffer.clear();
    }
    output_line += match output.close().await {
        Ok(sent) => sent,
        Err(err) => return exit_with_error(err),
    };
    if !quiet {
        println!(
            "Piped {} of {} docs to {output_name} in {:.3} seconds",
            comma_formatted(output_line),
            comma_formatted(input_line),
            start_time.elapsed().as_secs_f32()
        );
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

fn parse_nonzero_usize(value: &str) -> Result<usize, String> {
    let parsed = value.parse::<usize>().map_err(|err| err.to_string())?;
    if parsed == 0 {
        return Err("value must be at least 1".to_string());
    }
    Ok(parsed)
}
