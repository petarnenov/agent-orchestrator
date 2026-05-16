use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::runner::{AgentKind, AgentSelection, Phase};

#[derive(Debug, Clone)]
pub struct RunArtifacts {
    pub run_id: String,
    pub run_dir: PathBuf,
    pub output_dir: PathBuf,
    pub summary_path: PathBuf,
}

impl RunArtifacts {
    pub fn create(output_root: &Path, run_name: Option<&str>) -> Result<Self> {
        fs::create_dir_all(output_root)
            .with_context(|| format!("failed to create {}", output_root.display()))?;

        let run_id = Uuid::new_v4().to_string();
        let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
        let label = run_name
            .map(sanitize_label)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "run".to_string());
        let run_dir = output_root.join(format!(
            "{timestamp}-{label}-{short_id}",
            short_id = &run_id[..8]
        ));

        fs::create_dir_all(&run_dir)
            .with_context(|| format!("failed to create {}", run_dir.display()))?;

        Ok(Self {
            run_id,
            output_dir: run_dir.clone(),
            summary_path: run_dir.join("run-summary.json"),
            run_dir,
        })
    }

    pub fn output_path(&self, phase: Phase) -> PathBuf {
        match phase {
            Phase::Prospect1 => self.run_dir.join("prospect1.md"),
            Phase::Prospect2 => self.run_dir.join("prospect2.md"),
            Phase::Synthesis => self.run_dir.join("plan.md"),
            Phase::Implementation => self.run_dir.join("implementation-report.md"),
        }
    }

    pub fn prompt_path(&self, phase: Phase) -> PathBuf {
        self.run_dir.join(format!("{}.prompt.md", phase.slug()))
    }

    pub fn stdout_log_path(&self, phase: Phase) -> PathBuf {
        self.run_dir.join(format!("{}.stdout.log", phase.slug()))
    }

    pub fn stderr_log_path(&self, phase: Phase) -> PathBuf {
        self.run_dir.join(format!("{}.stderr.log", phase.slug()))
    }

    pub fn persist_summary(&self, summary: &RunSummary) -> Result<()> {
        let content =
            serde_json::to_string_pretty(summary).context("failed to serialize run summary")?;
        fs::write(&self.summary_path, content)
            .with_context(|| format!("failed to write {}", self.summary_path.display()))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub run_id: String,
    pub status: RunStatus,
    pub task_file: PathBuf,
    pub working_dir: PathBuf,
    pub output_dir: PathBuf,
    pub selected_agents: AgentSelection,
    pub heartbeat_interval_seconds: u64,
    pub total_phases: usize,
    pub completed_phases: usize,
    pub current_phase: Option<Phase>,
    pub current_phases: Vec<Phase>,
    pub progress_percent: u8,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub phases: Vec<PhaseSummary>,
}

impl RunSummary {
    pub fn new(
        run_id: String,
        task_file: PathBuf,
        working_dir: PathBuf,
        output_dir: PathBuf,
        selected_agents: AgentSelection,
    ) -> Self {
        Self {
            run_id,
            status: RunStatus::Running,
            task_file,
            working_dir,
            output_dir,
            selected_agents,
            heartbeat_interval_seconds: 5,
            total_phases: Phase::ALL.len(),
            completed_phases: 0,
            current_phase: None,
            current_phases: Vec::new(),
            progress_percent: 0,
            started_at: Utc::now(),
            finished_at: None,
            phases: Vec::new(),
        }
    }

    pub fn begin_phase(
        &mut self,
        phase: Phase,
        agent: AgentKind,
        prompt_file: PathBuf,
        output_file: PathBuf,
        stdout_log: PathBuf,
        stderr_log: PathBuf,
    ) {
        self.phases.push(PhaseSummary {
            phase,
            agent,
            status: PhaseStatus::Running,
            prompt_file,
            output_file,
            stdout_log,
            stderr_log,
            exit_code: None,
            started_at: Utc::now(),
            last_activity_at: None,
            last_activity_message: None,
            finished_at: None,
            error: None,
        });
        self.refresh_progress();
    }

    pub fn update_phase_activity(&mut self, phase: Phase, message: impl Into<String>) {
        if let Some(record) = self.phases.iter_mut().find(|record| record.phase == phase) {
            record.last_activity_at = Some(Utc::now());
            record.last_activity_message = Some(message.into());
        }
    }

    pub fn complete_phase(&mut self, phase: Phase, exit_code: Option<i32>) {
        if let Some(record) = self.phases.iter_mut().find(|record| record.phase == phase) {
            record.status = PhaseStatus::Completed;
            record.exit_code = exit_code;
            record.finished_at = Some(Utc::now());
        }
        self.refresh_progress();
    }

    pub fn fail_phase(&mut self, phase: Phase, exit_code: Option<i32>, error: String) {
        if let Some(record) = self.phases.iter_mut().find(|record| record.phase == phase) {
            record.status = PhaseStatus::Failed;
            record.exit_code = exit_code;
            record.error = Some(error);
            record.finished_at = Some(Utc::now());
        }
        self.refresh_progress();
        self.status = RunStatus::Failed;
        self.finished_at = Some(Utc::now());
    }

    pub fn complete_run(&mut self) {
        self.status = RunStatus::Completed;
        self.current_phase = None;
        self.current_phases.clear();
        self.completed_phases = self.total_phases;
        self.progress_percent = 100;
        self.finished_at = Some(Utc::now());
    }

    pub fn fail_run(&mut self) {
        self.status = RunStatus::Failed;
        self.current_phase = None;
        self.current_phases.clear();
        self.finished_at = Some(Utc::now());
    }

    fn refresh_progress(&mut self) {
        self.total_phases = Phase::ALL.len();
        self.completed_phases = self
            .phases
            .iter()
            .filter(|phase| matches!(phase.status, PhaseStatus::Completed))
            .count();
        self.current_phases = self
            .phases
            .iter()
            .filter(|phase| matches!(phase.status, PhaseStatus::Running))
            .map(|phase| phase.phase)
            .collect();
        self.current_phase = self.current_phases.last().copied();
        self.progress_percent = if self.total_phases == 0 {
            0
        } else {
            ((self.completed_phases * 100) / self.total_phases) as u8
        };
    }
}

#[derive(Debug, Clone)]
pub struct ActivePhaseStatus {
    pub phase: Phase,
    pub agent: AgentKind,
    pub elapsed_seconds: u64,
    pub last_activity_seconds: Option<u64>,
    pub last_activity_message: Option<String>,
}

impl RunSummary {
    pub fn active_phase_statuses(&self, now: DateTime<Utc>) -> Vec<ActivePhaseStatus> {
        self.phases
            .iter()
            .filter(|phase| matches!(phase.status, PhaseStatus::Running))
            .map(|phase| ActivePhaseStatus {
                phase: phase.phase,
                agent: phase.agent,
                elapsed_seconds: elapsed_seconds(phase.started_at, now),
                last_activity_seconds: phase
                    .last_activity_at
                    .map(|last_activity| elapsed_seconds(last_activity, now)),
                last_activity_message: phase.last_activity_message.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhaseSummary {
    pub phase: Phase,
    pub agent: AgentKind,
    pub status: PhaseStatus,
    pub prompt_file: PathBuf,
    pub output_file: PathBuf,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
    pub exit_code: Option<i32>,
    pub started_at: DateTime<Utc>,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub last_activity_message: Option<String>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

fn elapsed_seconds(started_at: DateTime<Utc>, now: DateTime<Utc>) -> u64 {
    now.signed_duration_since(started_at).num_seconds().max(0) as u64
}

fn sanitize_label(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => ch.to_ascii_lowercase(),
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_predictable_artifact_names() {
        let temp = tempdir().unwrap();
        let artifacts = RunArtifacts::create(temp.path(), Some("My Run")).unwrap();

        assert_eq!(
            artifacts.output_path(Phase::Prospect1).file_name().unwrap(),
            "prospect1.md"
        );
        assert_eq!(
            artifacts.prompt_path(Phase::Synthesis).file_name().unwrap(),
            "synthesis.prompt.md"
        );
        assert!(artifacts.output_dir.exists());
    }
}
