mod client;
mod target;

use clap::Parser;
use client::Auth;
use fluent_uri::UriRef;
use serde_json::Value;
use std::io::BufRead;
use target::{Input, Output};

#[derive(Parser)]
struct Cli {
    /// The input to read docs from
    #[arg(help = "The input to read docs from")]
    input: UriRef<String>,
    /// The output to send docs to
    #[arg(help = "The output to send docs to")]
    output: UriRef<String>,
    /// Authentication method to use (none, basic, apikey, etc.)
    #[arg(
        default_value = "none",
        help = "Authentication method to use (none, basic, apikey, etc.)",
        long
    )]
    /// Accept invalid certificates
    #[arg(
        help = "Ignore certificate validation",
        long,
        short = 'k',
        default_value = "false"
    )]
    insecure: bool,
    /// ApiKey for authentication
    #[arg(help = "Apikey to pass in http header ", long, short)]
    apikey: Option<String>,
    /// Username for authentication
    #[arg(
        help = "Username for authentication",
        long,
        short,
        conflicts_with = "apikey"
    )]
    username: Option<String>,
    /// Password for authentication
    #[arg(
        help = "Password for authentication",
        long,
        short,
        conflicts_with = "apikey"
    )]
    password: Option<String>,
    /// Quiet mode, don't print summary line
    #[arg(
        help = "Quiet mode, don't print summary line",
        long,
        short = 'q',
        default_value = "false"
    )]
    quiet: bool,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let start_time = std::time::Instant::now();
    // Initialize logger
    let env = env_logger::Env::default().filter_or("LOG_LEVEL", "info");
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
    } = args;

    let auth = Auth::try_new(username, password, apikey).expect("invalid authentication");

    let input = Input::try_from(input).expect("invalid input");
    log::debug!("input: {input}");

    let mut output = Output::try_new(insecure, auth, output).expect("invalid output");
    log::debug!("output: {output}");

    let mut input_line: usize = 0;
    let mut output_line: usize = 0;
    let mut reader = input.get_reader().expect("failed to get reader");
    let mut line_buffer = String::new();
    while let Ok(_) = reader.read_line(&mut line_buffer) {
        output_line += match serde_json::from_str::<Value>(&line_buffer) {
            Ok(json) => output.send(&json).await.expect("output send error"),
            Err(error) if serde_json::Error::is_eof(&error) => break,
            Err(error) => {
                log::error!("Failed to parse line {input_line}: {error}");
                0
            }
        };
        input_line += 1;
        line_buffer.clear();
    }
    let output_name = format!("{output}");
    output_line += output.close().await.expect("output close error");
    if !quiet {
        println!(
            "Piped {} of {} docs to {output_name} in {:.3} seconds",
            comma_formatted(input_line),
            comma_formatted(output_line),
            start_time.elapsed().as_secs_f32()
        );
    }
}

fn comma_formatted(n: usize) -> String {
    let string = n.to_string();
    let len = string.len();
    let mut result = String::new();

    for (i, c) in string.chars().enumerate() {
        result.push(c);
        let pos = len - i - 1;
        if pos > 0 && pos % 3 == 0 {
            result.push(',');
        }
    }

    result.chars().collect()
}
