use std::env;
use std::ffi::OsStr;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;

use crate::runner::{AgentCliConfig, AgentKind, AgentSelection, ExecutionMode, Phase};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "CLI orchestrator for Copilot/Claude execution pipelines"
)]
pub struct Cli {
    #[arg(
        value_name = "TASK_FILE",
        help = "Path to a .md or .txt task description"
    )]
    task_file: PathBuf,

    #[arg(
        long,
        default_value = ".",
        help = "Workspace root where agents will run"
    )]
    working_dir: PathBuf,

    #[arg(
        long,
        help = "Directory for run artifacts. Defaults to <working-dir>/.agent-orchestrator-runs"
    )]
    output_dir: Option<PathBuf>,

    #[arg(long, default_value = "prompt-brainstorm.md")]
    brainstorm_prompt: PathBuf,

    #[arg(long, default_value = "prompt-synthesis.md")]
    synthesis_prompt: PathBuf,

    #[arg(long, default_value = "prompt-implementation.md")]
    implementation_prompt: PathBuf,

    #[arg(long, default_value = "copilot")]
    copilot_bin: String,

    #[arg(long = "copilot-arg")]
    copilot_args: Vec<String>,

    #[arg(long, default_value = "claude")]
    claude_bin: String,

    #[arg(long = "claude-arg")]
    claude_args: Vec<String>,

    #[arg(long, help = "Optional label used in the generated run directory name")]
    run_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PromptPaths {
    pub brainstorm: PathBuf,
    pub synthesis: PathBuf,
    pub implementation: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ResolvedCli {
    pub task_file: PathBuf,
    pub working_dir: PathBuf,
    pub output_root: PathBuf,
    pub prompt_paths: PromptPaths,
    pub execution_mode: ExecutionMode,
    pub agent_selection: AgentSelection,
    pub copilot: AgentCliConfig,
    pub claude: AgentCliConfig,
    pub run_name: Option<String>,
}

impl Cli {
    pub fn resolve(self) -> Result<ResolvedCli> {
        let invocation_dir =
            env::current_dir().context("failed to resolve current working directory")?;
        let task_file = resolve_existing_file(&invocation_dir, &self.task_file, "task file")?;
        validate_task_file(&task_file)?;

        let working_dir =
            resolve_existing_dir(&invocation_dir, &self.working_dir, "working directory")?;
        let prompt_paths = PromptPaths {
            brainstorm: resolve_existing_file(
                &invocation_dir,
                &self.brainstorm_prompt,
                "brainstorm prompt file",
            )?,
            synthesis: resolve_existing_file(
                &invocation_dir,
                &self.synthesis_prompt,
                "synthesis prompt file",
            )?,
            implementation: resolve_existing_file(
                &invocation_dir,
                &self.implementation_prompt,
                "implementation prompt file",
            )?,
        };
        let output_root = resolve_output_root(&working_dir, self.output_dir)?;
        let (execution_mode, agent_selection) = resolve_run_configuration()?;

        Ok(ResolvedCli {
            task_file,
            working_dir,
            output_root,
            prompt_paths,
            execution_mode,
            agent_selection,
            copilot: AgentCliConfig::new(self.copilot_bin, self.copilot_args),
            claude: AgentCliConfig::new(self.claude_bin, self.claude_args),
            run_name: self.run_name,
        })
    }
}

fn resolve_existing_file(base: &Path, candidate: &Path, label: &str) -> Result<PathBuf> {
    let resolved = absolutize(base, candidate);
    let metadata = resolved
        .metadata()
        .with_context(|| format!("failed to access {label} at {}", resolved.display()))?;
    if !metadata.is_file() {
        bail!("{label} is not a file: {}", resolved.display());
    }
    resolved
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {label} at {}", resolved.display()))
}

fn resolve_existing_dir(base: &Path, candidate: &Path, label: &str) -> Result<PathBuf> {
    let resolved = absolutize(base, candidate);
    let metadata = resolved
        .metadata()
        .with_context(|| format!("failed to access {label} at {}", resolved.display()))?;
    if !metadata.is_dir() {
        bail!("{label} is not a directory: {}", resolved.display());
    }
    resolved
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {label} at {}", resolved.display()))
}

fn resolve_output_root(working_dir: &Path, candidate: Option<PathBuf>) -> Result<PathBuf> {
    let output_root = match candidate {
        Some(path) => absolutize(working_dir, &path),
        None => working_dir.join(".agent-orchestrator-runs"),
    };

    if output_root.exists() && !output_root.is_dir() {
        return Err(anyhow!(
            "output path exists but is not a directory: {}",
            output_root.display()
        ));
    }

    Ok(output_root)
}

fn absolutize(base: &Path, candidate: &Path) -> PathBuf {
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base.join(candidate)
    }
}

fn validate_task_file(task_file: &Path) -> Result<()> {
    let extension = task_file.extension().and_then(OsStr::to_str);
    match extension {
        Some("md") | Some("txt") => Ok(()),
        _ => bail!(
            "task file must have .md or .txt extension: {}",
            task_file.display()
        ),
    }
}

fn resolve_run_configuration() -> Result<(ExecutionMode, AgentSelection)> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        let mut reader = stdin.lock();
        let execution_mode = prompt_for_execution_mode(&mut reader, &mut stdout)?;
        let agent_selection = prompt_for_agent_selection(&mut reader, &mut stdout)?;
        Ok((execution_mode, agent_selection))
    } else {
        Ok((
            ExecutionMode::legacy_default(),
            AgentSelection::legacy_default(),
        ))
    }
}

fn prompt_for_execution_mode<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
) -> Result<ExecutionMode> {
    loop {
        writeln!(
            writer,
            "Select execution mode: [1] Plan only [2] Full implementation"
        )
        .context("failed to write execution mode prompt")?;
        write!(writer, "> ").context("failed to write prompt marker")?;
        writer.flush().context("failed to flush prompt marker")?;

        let mut input = String::new();
        let bytes = reader
            .read_line(&mut input)
            .context("failed to read execution mode selection")?;
        if bytes == 0 {
            bail!("interactive selection ended before execution mode was chosen");
        }

        match input.trim().to_ascii_lowercase().as_str() {
            "1" | "plan" | "plan-only" | "plan_only" => return Ok(ExecutionMode::PlanOnly),
            "2"
            | "implement"
            | "implementation"
            | "full"
            | "full-implementation"
            | "full_implementation" => return Ok(ExecutionMode::FullImplementation),
            _ => {
                writeln!(
                    writer,
                    "Please choose 1/plan-only or 2/full-implementation."
                )
                .context("failed to write validation message")?;
                writer
                    .flush()
                    .context("failed to flush validation message")?;
            }
        }
    }
}

fn prompt_for_agent_selection<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
) -> Result<AgentSelection> {
    writeln!(writer, "Select agent for each stage:").context("failed to write selection prompt")?;
    writer.flush().context("failed to flush selection prompt")?;

    Ok(AgentSelection {
        prospect1: prompt_for_agent(reader, writer, Phase::Prospect1)?,
        prospect2: prompt_for_agent(reader, writer, Phase::Prospect2)?,
        synthesis: prompt_for_agent(reader, writer, Phase::Synthesis)?,
        implementation: prompt_for_agent(reader, writer, Phase::Implementation)?,
    })
}

fn prompt_for_agent<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    phase: Phase,
) -> Result<AgentKind> {
    loop {
        writeln!(writer, "{} agent? [1] Copilot [2] Claude", phase.title())
            .with_context(|| format!("failed to write {} prompt", phase.slug()))?;
        write!(writer, "> ").context("failed to write prompt marker")?;
        writer.flush().context("failed to flush prompt marker")?;

        let mut input = String::new();
        let bytes = reader
            .read_line(&mut input)
            .with_context(|| format!("failed to read selection for {}", phase.slug()))?;
        if bytes == 0 {
            bail!(
                "interactive selection ended before {} was chosen",
                phase.slug()
            );
        }

        match input.trim().to_ascii_lowercase().as_str() {
            "1" | "copilot" => return Ok(AgentKind::Copilot),
            "2" | "claude" => return Ok(AgentKind::Claude),
            _ => {
                writeln!(writer, "Please choose 1/Copilot or 2/Claude.")
                    .context("failed to write validation message")?;
                writer
                    .flush()
                    .context("failed to flush validation message")?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn rejects_non_text_task_files() {
        let err = validate_task_file(Path::new("task.json")).unwrap_err();
        assert!(err.to_string().contains(".md or .txt"));
    }

    #[test]
    fn parses_agent_selection_prompt_answers() {
        let mut input = Cursor::new("1\n2\n2\n1\n");
        let mut output = Vec::new();

        let selection = prompt_for_agent_selection(&mut input, &mut output).unwrap();

        assert_eq!(selection.prospect1, AgentKind::Copilot);
        assert_eq!(selection.prospect2, AgentKind::Claude);
        assert_eq!(selection.synthesis, AgentKind::Claude);
        assert_eq!(selection.implementation, AgentKind::Copilot);
    }

    #[test]
    fn parses_execution_mode_prompt_answers() {
        let mut input = Cursor::new("1\n");
        let mut output = Vec::new();

        let mode = prompt_for_execution_mode(&mut input, &mut output).unwrap();

        assert_eq!(mode, ExecutionMode::PlanOnly);
    }
}
