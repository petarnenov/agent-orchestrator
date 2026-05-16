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

impl std::fmt::Display for AgentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Copilot => write!(f, "Copilot"),
            Self::Claude => write!(f, "Claude"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Prospect1,
    Prospect2,
    Synthesis,
    Implementation,
}

impl Phase {
    pub const ALL: [Phase; 4] = [
        Phase::Prospect1,
        Phase::Prospect2,
        Phase::Synthesis,
        Phase::Implementation,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Prospect1 => "prospect1",
            Self::Prospect2 => "prospect2",
            Self::Synthesis => "synthesis",
            Self::Implementation => "implementation",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Prospect1 => "Prospect 1",
            Self::Prospect2 => "Prospect 2",
            Self::Synthesis => "Synthesis",
            Self::Implementation => "Implementation",
        }
    }

    pub fn output_description(self) -> &'static str {
        match self {
            Self::Prospect1 => "proposal",
            Self::Prospect2 => "proposal",
            Self::Synthesis => "plan",
            Self::Implementation => "implementation report",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AgentSelection {
    pub prospect1: AgentKind,
    pub prospect2: AgentKind,
    pub synthesis: AgentKind,
    pub implementation: AgentKind,
}

impl AgentSelection {
    pub fn legacy_default() -> Self {
        Self {
            prospect1: AgentKind::Copilot,
            prospect2: AgentKind::Claude,
            synthesis: AgentKind::Claude,
            implementation: AgentKind::Copilot,
        }
    }

    pub fn for_phase(self, phase: Phase) -> AgentKind {
        match phase {
            Phase::Prospect1 => self.prospect1,
            Phase::Prospect2 => self.prospect2,
            Phase::Synthesis => self.synthesis,
            Phase::Implementation => self.implementation,
        }
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
