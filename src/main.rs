use anyhow::Result;
use clap::Parser;

use agent_orchestrator::cli::{Cli, ResolvedCommand};
use agent_orchestrator::pipeline::Pipeline;
use agent_orchestrator::progress::ConsoleProgressReporter;
use agent_orchestrator::prompt::PromptTemplates;
use agent_orchestrator::runner::{ClaudeCliRunner, CopilotCliRunner};

fn main() -> Result<()> {
    match Cli::parse().resolve()? {
        ResolvedCommand::Prompts => {
            print_bundled_prompts();
            Ok(())
        }
        ResolvedCommand::Run(cli) => {
            let copilot = CopilotCliRunner::new(cli.copilot.clone());
            let claude = ClaudeCliRunner::new(cli.claude.clone());
            let pipeline = Pipeline::new(&copilot, &claude);
            let mut reporter = ConsoleProgressReporter;

            pipeline.execute_with_reporter(&cli, &mut reporter)?;

            Ok(())
        }
    }
}

fn print_bundled_prompts() {
    let prompts = PromptTemplates::bundled();
    print_section("brainstorm", &prompts.brainstorm);
    print_section("synthesis", &prompts.synthesis);
    print_section("implementation", &prompts.implementation);
}

fn print_section(name: &str, content: &str) {
    println!("=== {name} ===\n{content}");
}
