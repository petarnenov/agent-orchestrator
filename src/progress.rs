use std::path::PathBuf;

use crate::artifacts::ActivePhaseStatus;
use crate::runner::{AgentKind, AgentSelection, ExecutionMode, Phase};

pub trait ProgressReporter {
    fn report(&mut self, event: ProgressEvent);
}

pub struct NoopProgressReporter;

impl ProgressReporter for NoopProgressReporter {
    fn report(&mut self, _event: ProgressEvent) {}
}

pub struct ConsoleProgressReporter;

impl ProgressReporter for ConsoleProgressReporter {
    fn report(&mut self, event: ProgressEvent) {
        match event {
            ProgressEvent::RunStarted {
                task_file,
                output_dir,
                execution_mode,
                total_phases,
                selected_agents,
            } => {
                println!("Starting task: {}", task_file.display());
                println!("Artifacts: {}", output_dir.display());
                println!("Execution mode: {execution_mode}");
                println!(
                    "Agent selection: prospect1={}, prospect2={}, synthesis={}, implementation={}",
                    selected_agents.prospect1,
                    selected_agents.prospect2,
                    selected_agents.synthesis,
                    selected_agents.implementation
                );
                println!("Progress: 0% (0/{total_phases})");
            }
            ProgressEvent::PhaseStarted {
                phase,
                agent,
                phase_index,
                total_phases,
                progress_percent,
            } => {
                println!(
                    "[{phase_index}/{total_phases} | {progress_percent}%] Running {} with {}...",
                    phase.title(),
                    agent
                );
            }
            ProgressEvent::Heartbeat {
                completed_phases,
                total_phases,
                progress_percent,
                active_phases,
            } => {
                if active_phases.is_empty() {
                    return;
                }

                let details = active_phases
                    .iter()
                    .map(format_active_phase)
                    .collect::<Vec<_>>()
                    .join(" | ");

                println!(
                    "[{completed_phases}/{total_phases} | {progress_percent}%] Still running: {details}"
                );
            }
            ProgressEvent::PhaseCompleted {
                phase,
                agent,
                phase_index,
                total_phases,
                progress_percent,
                output_path,
            } => {
                println!(
                    "[{phase_index}/{total_phases} | {progress_percent}%] Completed {} with {} -> {} ({})",
                    phase.title(),
                    agent,
                    output_path.display(),
                    phase.output_description()
                );
            }
            ProgressEvent::PhaseFailed {
                phase,
                agent,
                phase_index,
                total_phases,
                progress_percent,
                error,
            } => {
                eprintln!(
                    "[{phase_index}/{total_phases} | {progress_percent}%] Failed {} with {}: {error}",
                    phase.title(),
                    agent
                );
            }
            ProgressEvent::RunCompleted {
                output_dir,
                completed_phases,
                total_phases,
            } => {
                println!(
                    "Done. Progress: 100% ({completed_phases}/{total_phases}). Artifacts: {}",
                    output_dir.display()
                );
            }
            ProgressEvent::RunFailed {
                output_dir,
                completed_phases,
                total_phases,
                error,
            } => {
                eprintln!(
                    "Failed. Progress: {}% ({completed_phases}/{total_phases}). Artifacts: {}. {}",
                    progress_percent(completed_phases, total_phases),
                    output_dir.display(),
                    error
                );
            }
        }
    }
}

pub enum ProgressEvent {
    RunStarted {
        task_file: PathBuf,
        output_dir: PathBuf,
        execution_mode: ExecutionMode,
        total_phases: usize,
        selected_agents: AgentSelection,
    },
    PhaseStarted {
        phase: Phase,
        agent: AgentKind,
        phase_index: usize,
        total_phases: usize,
        progress_percent: u8,
    },
    Heartbeat {
        completed_phases: usize,
        total_phases: usize,
        progress_percent: u8,
        active_phases: Vec<ActivePhaseStatus>,
    },
    PhaseCompleted {
        phase: Phase,
        agent: AgentKind,
        phase_index: usize,
        total_phases: usize,
        progress_percent: u8,
        output_path: PathBuf,
    },
    PhaseFailed {
        phase: Phase,
        agent: AgentKind,
        phase_index: usize,
        total_phases: usize,
        progress_percent: u8,
        error: String,
    },
    RunCompleted {
        output_dir: PathBuf,
        completed_phases: usize,
        total_phases: usize,
    },
    RunFailed {
        output_dir: PathBuf,
        completed_phases: usize,
        total_phases: usize,
        error: String,
    },
}

fn format_active_phase(active_phase: &ActivePhaseStatus) -> String {
    let last_activity = match (
        active_phase.last_activity_seconds,
        active_phase.last_activity_message.as_deref(),
    ) {
        (Some(seconds), Some(message)) => {
            format!(
                "last activity {} ago: {}",
                format_duration(seconds),
                message
            )
        }
        (Some(seconds), None) => format!("last activity {} ago", format_duration(seconds)),
        (None, Some(message)) => format!("last activity: {}", message),
        (None, None) => "last activity: unknown".to_string(),
    };

    format!(
        "{} with {} elapsed {} ({})",
        active_phase.phase.title(),
        active_phase.agent,
        format_duration(active_phase.elapsed_seconds),
        last_activity
    )
}

fn format_duration(total_seconds: u64) -> String {
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

fn progress_percent(completed_phases: usize, total_phases: usize) -> u8 {
    if total_phases == 0 {
        0
    } else {
        ((completed_phases * 100) / total_phases) as u8
    }
}
