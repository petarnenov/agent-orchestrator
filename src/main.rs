use anyhow::Result;
use clap::Parser;

use agent_orchestrator::cli::Cli;
use agent_orchestrator::pipeline::Pipeline;
use agent_orchestrator::progress::ConsoleProgressReporter;
use agent_orchestrator::runner::{ClaudeCliRunner, CopilotCliRunner};

fn main() -> Result<()> {
    let cli = Cli::parse().resolve()?;
    let copilot = CopilotCliRunner::new(cli.copilot.clone());
    let claude = ClaudeCliRunner::new(cli.claude.clone());
    let pipeline = Pipeline::new(&copilot, &claude);
    let mut reporter = ConsoleProgressReporter;

    pipeline.execute_with_reporter(&cli, &mut reporter)?;

    Ok(())
}
