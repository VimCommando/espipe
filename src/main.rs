mod client;
mod input;
mod output;

use clap::Parser;
use client::Auth;
use fluent_uri::UriRef;
use input::Input;
use output::{BulkAction, ElasticsearchOutputConfig, Output};
use std::process::ExitCode;

#[derive(Parser)]
struct Cli {
    /// The input to read docs from
    #[arg(help = "The input URI to read docs from")]
    input: UriRef<String>,
    /// The output to send docs to
    #[arg(help = "The output URI to send docs to")]
    output: UriRef<String>,
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
    /// Password for authentication
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
        default_value_t = 5_000usize,
        value_parser = parse_nonzero_usize
    )]
    batch_size: usize,
    /// Maximum concurrent Elasticsearch bulk requests
    #[arg(
        help = "Maximum concurrent Elasticsearch bulk requests",
        long,
        default_value_t = 16usize,
        value_parser = parse_nonzero_usize
    )]
    max_requests: usize,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 3)]
async fn main() -> ExitCode {
    let start_time = std::time::Instant::now();
    // Initialize logger
    let env = env_logger::Env::default().filter_or("LOG_LEVEL", "warn");
    env_logger::Builder::from_env(env)
        .format_timestamp_millis()
        .init();

    // Use clap to parse command line arguments
    let args = Cli::parse();
    let Cli {
        input,
        output,
        quiet,
        insecure,
        apikey,
        password,
        username,
        uncompressed,
        action,
        batch_size,
        max_requests,
    } = args;

    let auth = match Auth::try_new(apikey, username, password) {
        Ok(auth) => auth,
        Err(err) => return exit_with_error(err),
    };
    let elasticsearch_config = match ElasticsearchOutputConfig::try_new(batch_size, max_requests) {
        Ok(config) => config,
        Err(err) => return exit_with_error(err),
    };

    let mut input = match Input::try_new(input).await {
        Ok(input) => input,
        Err(err) => return exit_with_error(err),
    };
    log::debug!("input: {input}");

    let mut output = match Output::try_new(
        insecure,
        auth,
        output,
        action,
        !uncompressed,
        elasticsearch_config,
    ) {
        Ok(output) => output,
        Err(err) => return exit_with_error(err),
    };
    log::debug!("output: {output}");

    let mut input_line: usize = 0;
    let mut output_line: usize = 0;
    let mut line_buffer = String::with_capacity(1024);
    while let Ok(line) = input.read_line(&mut line_buffer) {
        input_line += 1;
        output_line += output.send(line).await.expect("output send error");
        line_buffer.clear();
    }
    let output_name = format!("{output}");
    output_line += output.close().await.expect("output close error");
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
