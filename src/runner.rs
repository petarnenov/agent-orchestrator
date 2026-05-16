use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct AgentCliConfig {
    pub program: String,
    pub extra_args: Vec<String>,
}

impl AgentCliConfig {
    pub fn new(program: String, extra_args: Vec<String>) -> Self {
        Self {
            program,
            extra_args,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Copilot,
    Claude,
}

impl AgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Copilot => "copilot",
            Self::Claude => "claude",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    BrainstormCopilot,
    BrainstormClaude,
    SynthesisClaude,
    ImplementationCopilot,
}

impl Phase {
    pub const ALL: [Phase; 4] = [
        Phase::BrainstormCopilot,
        Phase::BrainstormClaude,
        Phase::SynthesisClaude,
        Phase::ImplementationCopilot,
    ];

    pub fn agent(self) -> AgentKind {
        match self {
            Self::BrainstormCopilot | Self::ImplementationCopilot => AgentKind::Copilot,
            Self::BrainstormClaude | Self::SynthesisClaude => AgentKind::Claude,
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::BrainstormCopilot => "brainstorm-copilot",
            Self::BrainstormClaude => "brainstorm-claude",
            Self::SynthesisClaude => "synthesis-claude",
            Self::ImplementationCopilot => "implementation-copilot",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::BrainstormCopilot => "Copilot brainstorming",
            Self::BrainstormClaude => "Claude brainstorming",
            Self::SynthesisClaude => "Claude synthesis",
            Self::ImplementationCopilot => "Copilot implementation",
        }
    }

    pub fn output_description(self) -> &'static str {
        match self {
            Self::BrainstormCopilot => "proposal",
            Self::BrainstormClaude => "proposal",
            Self::SynthesisClaude => "plan",
            Self::ImplementationCopilot => "implementation report",
        }
    }

    pub fn position(self) -> usize {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .map(|index| index + 1)
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
pub struct AgentRequest {
    pub phase: Phase,
    pub prompt: String,
    pub working_dir: PathBuf,
    pub session_name: String,
}

#[derive(Debug, Clone)]
pub struct AgentResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
}

pub trait AgentRunner: Sync {
    fn run(&self, request: &AgentRequest) -> Result<AgentResult>;
}

#[derive(Debug, Clone)]
pub struct CopilotCliRunner {
    config: AgentCliConfig,
}

impl CopilotCliRunner {
    pub fn new(config: AgentCliConfig) -> Self {
        Self { config }
    }
}

impl AgentRunner for CopilotCliRunner {
    fn run(&self, request: &AgentRequest) -> Result<AgentResult> {
        let mut command = Command::new(&self.config.program);
        command
            .args(&self.config.extra_args)
            .arg("-C")
            .arg(&request.working_dir)
            .arg("--allow-all")
            .arg("--no-ask-user")
            .arg("--no-color")
            .arg("-s")
            .arg("--name")
            .arg(&request.session_name)
            .arg("-p")
            .arg(&request.prompt);

        let output = command
            .output()
            .with_context(|| format!("failed to launch {}", self.config.program))?;

        Ok(AgentResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code(),
            success: output.status.success(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeCliRunner {
    config: AgentCliConfig,
}

impl ClaudeCliRunner {
    pub fn new(config: AgentCliConfig) -> Self {
        Self { config }
    }
}

impl AgentRunner for ClaudeCliRunner {
    fn run(&self, request: &AgentRequest) -> Result<AgentResult> {
        let mut command = Command::new(&self.config.program);
        command
            .current_dir(&request.working_dir)
            .args(&self.config.extra_args)
            .arg("--print")
            .arg("--output-format")
            .arg("text")
            .arg("--no-session-persistence")
            .arg("--dangerously-skip-permissions")
            .arg("--add-dir")
            .arg(&request.working_dir)
            .arg("--name")
            .arg(&request.session_name)
            .arg(&request.prompt);

        let output = command
            .output()
            .with_context(|| format!("failed to launch {}", self.config.program))?;

        Ok(AgentResult {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code(),
            success: output.status.success(),
        })
    }
}
