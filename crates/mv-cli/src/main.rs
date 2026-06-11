//! mv-cli entry point: argument parsing fallback, output contract, and
//! command dispatch. Provider logic lives in `providers`, executors in
//! `executors`, telemetry wiring in `telemetry`.

mod cli;
mod commands;
mod executors;
mod providers;
mod telemetry;

use clap::Parser;

use cli::{Cli, Commands, WorkflowAction};
use providers::CompletionOutcome;

fn print_success(outcome: &CompletionOutcome, json: bool) {
    if json {
        let obj = serde_json::json!({
            "response": outcome.text,
            "model_used": outcome.model_used,
        });
        println!("{}", obj);
    } else {
        print!("{}", outcome.text);
    }
}

fn print_error(err: &mv_core::MvError, json: bool) {
    // Errors always go to stderr — `--json` only changes the shape, not the
    // channel, so `mv-cli --json ... | jq .response` never sees an error
    // object on the success stream.
    if json {
        let obj = serde_json::json!({ "error": err.to_string() });
        eprintln!("{}", obj);
    } else {
        eprintln!("Error: {err}");
    }
}

#[tokio::main]
async fn main() {
    // Try parsing with subcommands first; fall back to treating the first arg as a prompt
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            // If clap fails because the user typed `mv-cli "some prompt"` (no subcommand),
            // re-parse treating the first positional as a prompt subcommand
            if e.kind() == clap::error::ErrorKind::InvalidSubcommand
                || e.kind() == clap::error::ErrorKind::UnknownArgument
            {
                // Rebuild args: insert "prompt" as the subcommand
                let mut args: Vec<String> = std::env::args().collect();
                args.insert(1, "prompt".to_string());
                match Cli::try_parse_from(&args) {
                    Ok(cli) => cli,
                    Err(e2) => {
                        e2.exit();
                    }
                }
            } else {
                e.exit();
            }
        }
    };

    telemetry::init_tracing(cli.verbose, cli.otlp.as_deref());

    let result = match &cli.command {
        Some(Commands::Prompt(args)) => {
            let effective_stream = if args.stream && cli.json {
                eprintln!(
                    "warning: --json overrides --stream; falling back to buffered JSON output"
                );
                false
            } else {
                args.stream
            };
            match commands::prompt::run_prompt(args, cli.json, effective_stream).await {
                Ok(outcome) => {
                    print_success(&outcome, cli.json);
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }
        None => {
            // No subcommand and no prompt — show help
            eprintln!("Error: no prompt provided. Use: mv-cli <PROMPT> or mv-cli prompt <PROMPT>");
            telemetry::shutdown_tracing();
            std::process::exit(2);
        }
        Some(Commands::Workflow { action }) => match action {
            WorkflowAction::Run(args) => commands::workflow::run_workflow(args, cli.json).await,
            WorkflowAction::Validate(args) => commands::workflow::run_workflow_validate(args).await,
        },
    };

    if let Err(err) = result {
        print_error(&err, cli.json);
        telemetry::shutdown_tracing();
        std::process::exit(1);
    }

    telemetry::shutdown_tracing();
}
