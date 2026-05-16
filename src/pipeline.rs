use std::fs;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::artifacts::{RunArtifacts, RunSummary};
use crate::cli::ResolvedCli;
use crate::progress::{NoopProgressReporter, ProgressEvent, ProgressReporter};
use crate::prompt::{PromptContext, PromptTemplates, render_prompt};
use crate::runner::{AgentKind, AgentRequest, AgentRunner, Phase};

pub struct Pipeline<'a> {
    copilot: &'a dyn AgentRunner,
    claude: &'a dyn AgentRunner,
    heartbeat_interval: Duration,
}

#[derive(Clone)]
struct PhaseExecution {
    phase: Phase,
    phase_index: usize,
    request: AgentRequest,
    output_path: std::path::PathBuf,
    prompt_path: std::path::PathBuf,
    stdout_log: std::path::PathBuf,
    stderr_log: std::path::PathBuf,
}

impl<'a> Pipeline<'a> {
    pub fn new(copilot: &'a dyn AgentRunner, claude: &'a dyn AgentRunner) -> Self {
        Self::with_heartbeat_interval(copilot, claude, Duration::from_secs(5))
    }

    pub fn with_heartbeat_interval(
        copilot: &'a dyn AgentRunner,
        claude: &'a dyn AgentRunner,
        heartbeat_interval: Duration,
    ) -> Self {
        Self {
            copilot,
            claude,
            heartbeat_interval,
        }
    }

    pub fn execute(&self, cli: &ResolvedCli) -> Result<RunSummary> {
        let mut reporter = NoopProgressReporter;
        self.execute_with_reporter(cli, &mut reporter)
    }

    pub fn execute_with_reporter(
        &self,
        cli: &ResolvedCli,
        reporter: &mut dyn ProgressReporter,
    ) -> Result<RunSummary> {
        let task_content = fs::read_to_string(&cli.task_file)
            .with_context(|| format!("failed to read {}", cli.task_file.display()))?;
        let templates = PromptTemplates::load(&cli.prompt_paths)?;
        let artifacts =
            RunArtifacts::create(&cli.output_root, &cli.working_dir, cli.run_name.as_deref())?;
        let mut summary = RunSummary::new(
            artifacts.run_id.clone(),
            cli.task_file.clone(),
            cli.working_dir.clone(),
            artifacts.output_dir.clone(),
            cli.execution_mode,
            cli.agent_selection,
        );
        summary.heartbeat_interval_seconds = self.heartbeat_interval.as_secs();
        artifacts.persist_summary(&summary)?;
        reporter.report(ProgressEvent::RunStarted {
            task_file: cli.task_file.clone(),
            output_dir: artifacts.output_dir.clone(),
            execution_mode: cli.execution_mode,
            total_phases: summary.total_phases,
            selected_agents: cli.agent_selection,
        });

        let result = self.execute_inner(
            cli,
            &task_content,
            &templates,
            &artifacts,
            &mut summary,
            reporter,
        );
        if result.is_ok() {
            summary.complete_run();
            reporter.report(ProgressEvent::RunCompleted {
                output_dir: summary.output_dir.clone(),
                completed_phases: summary.completed_phases,
                total_phases: summary.total_phases,
            });
        } else {
            if !matches!(summary.status, crate::artifacts::RunStatus::Failed) {
                summary.fail_run();
            }
            reporter.report(ProgressEvent::RunFailed {
                output_dir: summary.output_dir.clone(),
                completed_phases: summary.completed_phases,
                total_phases: summary.total_phases,
                error: result
                    .as_ref()
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "pipeline execution failed".to_string()),
            });
        }
        artifacts.persist_summary(&summary)?;
        result?;

        Ok(summary)
    }

    fn execute_inner(
        &self,
        cli: &ResolvedCli,
        task_content: &str,
        templates: &PromptTemplates,
        artifacts: &RunArtifacts,
        summary: &mut RunSummary,
        reporter: &mut dyn ProgressReporter,
    ) -> Result<()> {
        self.run_parallel_phases(
            cli,
            task_content,
            templates,
            artifacts,
            summary,
            &[Phase::Prospect1, Phase::Prospect2],
            reporter,
        )?;
        self.run_phase(
            cli,
            task_content,
            templates,
            artifacts,
            summary,
            Phase::Synthesis,
            reporter,
        )?;
        if cli.execution_mode.includes_implementation() {
            self.run_phase(
                cli,
                task_content,
                templates,
                artifacts,
                summary,
                Phase::Implementation,
                reporter,
            )?;
        } else {
            self.prepare_phase_execution(
                cli,
                task_content,
                templates,
                artifacts,
                Phase::Implementation,
            )?;
        }

        Ok(())
    }

    fn run_phase(
        &self,
        cli: &ResolvedCli,
        task_content: &str,
        templates: &PromptTemplates,
        artifacts: &RunArtifacts,
        summary: &mut RunSummary,
        phase: Phase,
        reporter: &mut dyn ProgressReporter,
    ) -> Result<()> {
        let execution =
            self.prepare_phase_execution(cli, task_content, templates, artifacts, phase)?;
        self.start_phase(cli, &execution, summary, artifacts, reporter)?;
        summary.update_phase_activity(phase, "agent process started");
        artifacts.persist_summary(summary)?;

        thread::scope(|scope| {
            let (sender, receiver) = mpsc::channel();
            let runner = self.runner_for(cli.agent_selection.for_phase(phase));
            let request = execution.request.clone();
            scope.spawn(move || {
                let result = runner
                    .run(&request)
                    .with_context(|| format!("{} phase failed to start", phase.slug()));
                let _ = sender.send(result);
            });

            summary.update_phase_activity(phase, "waiting for agent output");
            artifacts.persist_summary(summary)?;

            let result = self.wait_for_single_phase_result(&receiver, summary, reporter);
            self.finalize_phase_execution(&execution, result, summary, artifacts, reporter)
        })
    }

    fn runner_for(&self, agent: AgentKind) -> &dyn AgentRunner {
        match agent {
            AgentKind::Copilot => self.copilot,
            AgentKind::Claude => self.claude,
        }
    }

    fn run_parallel_phases(
        &self,
        cli: &ResolvedCli,
        task_content: &str,
        templates: &PromptTemplates,
        artifacts: &RunArtifacts,
        summary: &mut RunSummary,
        phases: &[Phase],
        reporter: &mut dyn ProgressReporter,
    ) -> Result<()> {
        let executions = phases
            .iter()
            .map(|phase| {
                self.prepare_phase_execution(cli, task_content, templates, artifacts, *phase)
            })
            .collect::<Result<Vec<_>>>()?;

        for execution in &executions {
            self.start_phase(cli, execution, summary, artifacts, reporter)?;
            summary.update_phase_activity(execution.phase, "agent process started");
        }
        artifacts.persist_summary(summary)?;

        let (sender, receiver) = mpsc::channel();
        thread::scope(|scope| {
            for execution in &executions {
                let sender = sender.clone();
                let runner = self.runner_for(cli.agent_selection.for_phase(execution.phase));
                let request = execution.request.clone();
                let phase = execution.phase;
                scope.spawn(move || {
                    let result = runner
                        .run(&request)
                        .with_context(|| format!("{} phase failed to start", phase.slug()));
                    let _ = sender.send((phase, result));
                });
            }

            for execution in &executions {
                summary.update_phase_activity(execution.phase, "waiting for agent output");
            }
            artifacts.persist_summary(summary)?;

            drop(sender);

            let mut first_error = None;
            let mut remaining_results = executions.len();
            while remaining_results > 0 {
                let (phase, result) =
                    self.wait_for_parallel_phase_results(&receiver, summary, reporter)?;
                let execution = executions
                    .iter()
                    .find(|execution| execution.phase == phase)
                    .expect("phase execution should exist");
                if let Err(error) =
                    self.finalize_phase_execution(execution, result, summary, artifacts, reporter)
                {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
                remaining_results -= 1;
            }

            if let Some(error) = first_error {
                Err(error)
            } else {
                Ok(())
            }
        })
    }

    fn prepare_phase_execution(
        &self,
        cli: &ResolvedCli,
        task_content: &str,
        templates: &PromptTemplates,
        artifacts: &RunArtifacts,
        phase: Phase,
    ) -> Result<PhaseExecution> {
        let output_path = artifacts.output_path(phase);
        let prompt_path = artifacts.prompt_path(phase, cli.execution_mode);
        let prompt_context = PromptContext {
            task_file_path: cli.task_file.clone(),
            task_content: task_content.to_string(),
            workspace_dir: cli.working_dir.clone(),
            target_output_path: output_path.clone(),
            prospect1_path: artifacts.output_path(Phase::Prospect1),
            prospect2_path: artifacts.output_path(Phase::Prospect2),
            plan_path: artifacts.plan_path(),
        };
        let prompt = render_prompt(phase, templates, &prompt_context);
        fs::write(&prompt_path, &prompt)
            .with_context(|| format!("failed to write {}", prompt_path.display()))?;

        Ok(PhaseExecution {
            phase,
            phase_index: phase.position(),
            request: AgentRequest {
                phase,
                prompt,
                working_dir: cli.working_dir.clone(),
                session_name: format!("agent-orchestrator-{}", phase.slug()),
            },
            output_path,
            prompt_path,
            stdout_log: artifacts.stdout_log_path(phase),
            stderr_log: artifacts.stderr_log_path(phase),
        })
    }

    fn start_phase(
        &self,
        cli: &ResolvedCli,
        execution: &PhaseExecution,
        summary: &mut RunSummary,
        artifacts: &RunArtifacts,
        reporter: &mut dyn ProgressReporter,
    ) -> Result<()> {
        let agent = cli.agent_selection.for_phase(execution.phase);
        summary.begin_phase(
            execution.phase,
            agent,
            execution.prompt_path.clone(),
            execution.output_path.clone(),
            execution.stdout_log.clone(),
            execution.stderr_log.clone(),
        );
        summary.update_phase_activity(execution.phase, "prompt prepared");
        artifacts.persist_summary(summary)?;
        reporter.report(ProgressEvent::PhaseStarted {
            phase: execution.phase,
            agent,
            phase_index: execution.phase_index,
            total_phases: summary.total_phases,
            progress_percent: summary.progress_percent,
        });
        Ok(())
    }

    fn finalize_phase_execution(
        &self,
        execution: &PhaseExecution,
        result: Result<crate::runner::AgentResult>,
        summary: &mut RunSummary,
        artifacts: &RunArtifacts,
        reporter: &mut dyn ProgressReporter,
    ) -> Result<()> {
        match result {
            Ok(result) => {
                summary.update_phase_activity(execution.phase, "agent output received");
                artifacts.persist_summary(summary)?;
                fs::write(&execution.stdout_log, &result.stdout).with_context(|| {
                    format!("failed to write {}", execution.stdout_log.display())
                })?;
                fs::write(&execution.stderr_log, &result.stderr).with_context(|| {
                    format!("failed to write {}", execution.stderr_log.display())
                })?;

                if !result.success {
                    let message = format_phase_error(
                        execution.phase,
                        &result.stdout,
                        &result.stderr,
                        result.exit_code,
                    );
                    summary.fail_phase(execution.phase, result.exit_code, message.clone());
                    artifacts.persist_summary(summary)?;
                    reporter.report(ProgressEvent::PhaseFailed {
                        phase: execution.phase,
                        agent: summary.selected_agents.for_phase(execution.phase),
                        phase_index: execution.phase_index,
                        total_phases: summary.total_phases,
                        progress_percent: summary.progress_percent,
                        error: message.clone(),
                    });
                    bail!("{message}");
                }

                let trimmed = result.stdout.trim();
                if trimmed.is_empty() {
                    let message = format!("{} returned empty output", execution.phase.slug());
                    summary.fail_phase(execution.phase, result.exit_code, message.clone());
                    artifacts.persist_summary(summary)?;
                    reporter.report(ProgressEvent::PhaseFailed {
                        phase: execution.phase,
                        agent: summary.selected_agents.for_phase(execution.phase),
                        phase_index: execution.phase_index,
                        total_phases: summary.total_phases,
                        progress_percent: summary.progress_percent,
                        error: message.clone(),
                    });
                    bail!("{message}");
                }

                fs::write(&execution.output_path, trimmed).with_context(|| {
                    format!("failed to write {}", execution.output_path.display())
                })?;
                summary.update_phase_activity(execution.phase, "artifact written");
                summary.complete_phase(execution.phase, result.exit_code);
                artifacts.persist_summary(summary)?;
                reporter.report(ProgressEvent::PhaseCompleted {
                    phase: execution.phase,
                    agent: summary.selected_agents.for_phase(execution.phase),
                    phase_index: execution.phase_index,
                    total_phases: summary.total_phases,
                    progress_percent: summary.progress_percent,
                    output_path: execution.output_path.clone(),
                });

                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                fs::write(&execution.stderr_log, &message).with_context(|| {
                    format!("failed to write {}", execution.stderr_log.display())
                })?;
                summary.fail_phase(execution.phase, None, message.clone());
                artifacts.persist_summary(summary)?;
                reporter.report(ProgressEvent::PhaseFailed {
                    phase: execution.phase,
                    agent: summary.selected_agents.for_phase(execution.phase),
                    phase_index: execution.phase_index,
                    total_phases: summary.total_phases,
                    progress_percent: summary.progress_percent,
                    error: message.clone(),
                });
                bail!("{message}");
            }
        }
    }

    fn wait_for_single_phase_result(
        &self,
        receiver: &mpsc::Receiver<Result<crate::runner::AgentResult>>,
        summary: &RunSummary,
        reporter: &mut dyn ProgressReporter,
    ) -> Result<crate::runner::AgentResult> {
        loop {
            match receiver.recv_timeout(self.heartbeat_interval) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) => self.emit_heartbeat(summary, reporter),
                Err(RecvTimeoutError::Disconnected) => {
                    bail!("phase worker exited before reporting a result")
                }
            }
        }
    }

    fn wait_for_parallel_phase_results(
        &self,
        receiver: &mpsc::Receiver<(Phase, Result<crate::runner::AgentResult>)>,
        summary: &RunSummary,
        reporter: &mut dyn ProgressReporter,
    ) -> Result<(Phase, Result<crate::runner::AgentResult>)> {
        loop {
            match receiver.recv_timeout(self.heartbeat_interval) {
                Ok(result) => return Ok(result),
                Err(RecvTimeoutError::Timeout) => self.emit_heartbeat(summary, reporter),
                Err(RecvTimeoutError::Disconnected) => {
                    bail!("parallel phase worker exited before reporting a result")
                }
            }
        }
    }

    fn emit_heartbeat(&self, summary: &RunSummary, reporter: &mut dyn ProgressReporter) {
        let active_phases = summary.active_phase_statuses(Utc::now());
        if active_phases.is_empty() {
            return;
        }

        reporter.report(ProgressEvent::Heartbeat {
            completed_phases: summary.completed_phases,
            total_phases: summary.total_phases,
            progress_percent: summary.progress_percent,
            active_phases,
        });
    }
}

fn format_phase_error(phase: Phase, stdout: &str, stderr: &str, exit_code: Option<i32>) -> String {
    let stdout_excerpt = excerpt(stdout);
    let stderr_excerpt = excerpt(stderr);
    format!(
        "{} failed with exit code {:?}. stderr: {} stdout: {}",
        phase.slug(),
        exit_code,
        stderr_excerpt,
        stdout_excerpt
    )
}

fn excerpt(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        "<empty>".to_string()
    } else {
        trimmed.chars().take(240).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use anyhow::anyhow;
    use tempfile::tempdir;

    use super::*;
    use crate::cli::{PromptPaths, ResolvedCli};
    use crate::runner::{AgentCliConfig, AgentResult, ExecutionMode};

    struct MockRunner {
        responses: Mutex<VecDeque<AgentResult>>,
    }

    impl MockRunner {
        fn new(responses: Vec<AgentResult>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
            }
        }
    }

    impl AgentRunner for MockRunner {
        fn run(&self, _request: &AgentRequest) -> Result<AgentResult> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow!("missing mock response"))
        }
    }

    struct RecordingReporter {
        events: Mutex<Vec<String>>,
    }

    impl RecordingReporter {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }
    }

    impl ProgressReporter for RecordingReporter {
        fn report(&mut self, event: ProgressEvent) {
            let label = match event {
                ProgressEvent::RunStarted { .. } => "run_started",
                ProgressEvent::PhaseStarted { phase, .. } => phase.slug(),
                ProgressEvent::Heartbeat { .. } => "heartbeat",
                ProgressEvent::PhaseCompleted { .. } => "phase_completed",
                ProgressEvent::PhaseFailed { .. } => "phase_failed",
                ProgressEvent::RunCompleted { .. } => "run_completed",
                ProgressEvent::RunFailed { .. } => "run_failed",
            };
            self.events.lock().unwrap().push(label.to_string());
        }
    }

    struct DelayedRunner {
        delay: Duration,
        inner: MockRunner,
    }

    impl DelayedRunner {
        fn new(delay: Duration, responses: Vec<AgentResult>) -> Self {
            Self {
                delay,
                inner: MockRunner::new(responses),
            }
        }
    }

    impl AgentRunner for DelayedRunner {
        fn run(&self, request: &AgentRequest) -> Result<AgentResult> {
            if matches!(request.phase, Phase::Prospect1 | Phase::Prospect2) {
                std::thread::sleep(self.delay);
            }
            self.inner.run(request)
        }
    }

    #[test]
    fn pipeline_writes_all_expected_artifacts() {
        let temp = tempdir().unwrap();
        let task_file = temp.path().join("task.md");
        let brainstorm = temp.path().join("prompt-brainstorm.md");
        let synthesis = temp.path().join("prompt-synthesis.md");
        let implementation = temp.path().join("prompt-implementation.md");

        fs::write(&task_file, "Build a CLI orchestrator").unwrap();
        fs::write(&brainstorm, "Brainstorm {{TASK_CONTENT}}").unwrap();
        fs::write(
            &synthesis,
            "Use {{COPILOT_PROPOSAL_PATH}} and {{CLAUDE_PROPOSAL_PATH}}",
        )
        .unwrap();
        fs::write(&implementation, "Implement from {{PLAN_PATH}}").unwrap();

        let cli = ResolvedCli {
            task_file: task_file.clone(),
            working_dir: temp.path().to_path_buf(),
            output_root: temp.path().join("runs"),
            prompt_paths: PromptPaths {
                brainstorm,
                synthesis,
                implementation,
            },
            execution_mode: ExecutionMode::FullImplementation,
            agent_selection: crate::runner::AgentSelection::legacy_default(),
            copilot: AgentCliConfig::new("copilot".to_string(), Vec::new()),
            claude: AgentCliConfig::new("claude".to_string(), Vec::new()),
            run_name: Some("test-run".to_string()),
        };
        let copilot = MockRunner::new(vec![
            AgentResult {
                stdout: "copilot proposal".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
            AgentResult {
                stdout: "implementation summary".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
        ]);
        let claude = MockRunner::new(vec![
            AgentResult {
                stdout: "claude proposal".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
            AgentResult {
                stdout: "final plan".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
        ]);

        let pipeline = Pipeline::new(&copilot, &claude);
        let summary = pipeline.execute(&cli).unwrap();

        assert!(summary.output_dir.join("prospect1.md").exists());
        assert!(summary.output_dir.join("prospect2.md").exists());
        assert!(temp.path().join("plan.md").exists());
        assert!(summary.output_dir.join("implementation-report.md").exists());
        assert!(summary.output_dir.join("run-summary.json").exists());
        assert_eq!(summary.execution_mode, ExecutionMode::FullImplementation);
        assert_eq!(summary.phases.len(), 4);
        assert_eq!(summary.completed_phases, 4);
        assert!(summary.current_phases.is_empty());
        assert_eq!(summary.heartbeat_interval_seconds, 5);
        assert_eq!(summary.progress_percent, 100);
    }

    #[test]
    fn pipeline_fails_on_empty_output() {
        let temp = tempdir().unwrap();
        let task_file = temp.path().join("task.md");
        let brainstorm = temp.path().join("prompt-brainstorm.md");
        let synthesis = temp.path().join("prompt-synthesis.md");
        let implementation = temp.path().join("prompt-implementation.md");

        fs::write(&task_file, "Build a CLI orchestrator").unwrap();
        fs::write(&brainstorm, "Brainstorm {{TASK_CONTENT}}").unwrap();
        fs::write(&synthesis, "Synthesize").unwrap();
        fs::write(&implementation, "Implement").unwrap();

        let cli = ResolvedCli {
            task_file,
            working_dir: temp.path().to_path_buf(),
            output_root: temp.path().join("runs"),
            prompt_paths: PromptPaths {
                brainstorm,
                synthesis,
                implementation,
            },
            execution_mode: ExecutionMode::FullImplementation,
            agent_selection: crate::runner::AgentSelection::legacy_default(),
            copilot: AgentCliConfig::new("copilot".to_string(), Vec::new()),
            claude: AgentCliConfig::new("claude".to_string(), Vec::new()),
            run_name: None,
        };
        let copilot = MockRunner::new(vec![AgentResult {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        }]);
        let claude = MockRunner::new(Vec::new());

        let pipeline = Pipeline::new(&copilot, &claude);
        let err = pipeline.execute(&cli).unwrap_err();

        assert!(err.to_string().contains("empty output"));
    }

    #[test]
    fn pipeline_reports_progress_events() {
        let temp = tempdir().unwrap();
        let task_file = temp.path().join("task.md");
        let brainstorm = temp.path().join("prompt-brainstorm.md");
        let synthesis = temp.path().join("prompt-synthesis.md");
        let implementation = temp.path().join("prompt-implementation.md");

        fs::write(&task_file, "Build a CLI orchestrator").unwrap();
        fs::write(&brainstorm, "Brainstorm {{TASK_CONTENT}}").unwrap();
        fs::write(&synthesis, "Synthesize").unwrap();
        fs::write(&implementation, "Implement").unwrap();

        let cli = ResolvedCli {
            task_file,
            working_dir: temp.path().to_path_buf(),
            output_root: temp.path().join("runs"),
            prompt_paths: PromptPaths {
                brainstorm,
                synthesis,
                implementation,
            },
            execution_mode: ExecutionMode::FullImplementation,
            agent_selection: crate::runner::AgentSelection::legacy_default(),
            copilot: AgentCliConfig::new("copilot".to_string(), Vec::new()),
            claude: AgentCliConfig::new("claude".to_string(), Vec::new()),
            run_name: None,
        };
        let copilot = MockRunner::new(vec![
            AgentResult {
                stdout: "copilot proposal".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
            AgentResult {
                stdout: "implementation summary".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
        ]);
        let claude = MockRunner::new(vec![
            AgentResult {
                stdout: "claude proposal".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
            AgentResult {
                stdout: "final plan".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
        ]);
        let pipeline = Pipeline::new(&copilot, &claude);
        let mut reporter = RecordingReporter::new();

        pipeline.execute_with_reporter(&cli, &mut reporter).unwrap();

        let events = reporter.events.lock().unwrap();
        assert!(events.iter().any(|event| event == "run_started"));
        assert!(events.iter().any(|event| event == "run_completed"));
        assert!(events.iter().any(|event| event == "prospect1"));
    }

    #[test]
    fn pipeline_emits_heartbeat_for_long_running_phases() {
        let temp = tempdir().unwrap();
        let task_file = temp.path().join("task.md");
        let brainstorm = temp.path().join("prompt-brainstorm.md");
        let synthesis = temp.path().join("prompt-synthesis.md");
        let implementation = temp.path().join("prompt-implementation.md");

        fs::write(&task_file, "Build a CLI orchestrator").unwrap();
        fs::write(&brainstorm, "Brainstorm {{TASK_CONTENT}}").unwrap();
        fs::write(&synthesis, "Synthesize").unwrap();
        fs::write(&implementation, "Implement").unwrap();

        let cli = ResolvedCli {
            task_file,
            working_dir: temp.path().to_path_buf(),
            output_root: temp.path().join("runs"),
            prompt_paths: PromptPaths {
                brainstorm,
                synthesis,
                implementation,
            },
            execution_mode: ExecutionMode::FullImplementation,
            agent_selection: crate::runner::AgentSelection::legacy_default(),
            copilot: AgentCliConfig::new("copilot".to_string(), Vec::new()),
            claude: AgentCliConfig::new("claude".to_string(), Vec::new()),
            run_name: None,
        };
        let copilot = DelayedRunner::new(
            Duration::from_millis(60),
            vec![
                AgentResult {
                    stdout: "copilot proposal".to_string(),
                    stderr: String::new(),
                    exit_code: Some(0),
                    success: true,
                },
                AgentResult {
                    stdout: "implementation summary".to_string(),
                    stderr: String::new(),
                    exit_code: Some(0),
                    success: true,
                },
            ],
        );
        let claude = DelayedRunner::new(
            Duration::from_millis(60),
            vec![
                AgentResult {
                    stdout: "claude proposal".to_string(),
                    stderr: String::new(),
                    exit_code: Some(0),
                    success: true,
                },
                AgentResult {
                    stdout: "final plan".to_string(),
                    stderr: String::new(),
                    exit_code: Some(0),
                    success: true,
                },
            ],
        );
        let pipeline =
            Pipeline::with_heartbeat_interval(&copilot, &claude, Duration::from_millis(20));
        let mut reporter = RecordingReporter::new();

        pipeline.execute_with_reporter(&cli, &mut reporter).unwrap();

        let events = reporter.events.lock().unwrap();
        assert!(events.iter().any(|event| event == "heartbeat"));
    }

    #[test]
    fn brainstorming_phases_run_in_parallel() {
        let temp = tempdir().unwrap();
        let task_file = temp.path().join("task.md");
        let brainstorm = temp.path().join("prompt-brainstorm.md");
        let synthesis = temp.path().join("prompt-synthesis.md");
        let implementation = temp.path().join("prompt-implementation.md");

        fs::write(&task_file, "Build a CLI orchestrator").unwrap();
        fs::write(&brainstorm, "Brainstorm {{TASK_CONTENT}}").unwrap();
        fs::write(&synthesis, "Synthesize").unwrap();
        fs::write(&implementation, "Implement").unwrap();

        let cli = ResolvedCli {
            task_file,
            working_dir: temp.path().to_path_buf(),
            output_root: temp.path().join("runs"),
            prompt_paths: PromptPaths {
                brainstorm,
                synthesis,
                implementation,
            },
            execution_mode: ExecutionMode::FullImplementation,
            agent_selection: crate::runner::AgentSelection::legacy_default(),
            copilot: AgentCliConfig::new("copilot".to_string(), Vec::new()),
            claude: AgentCliConfig::new("claude".to_string(), Vec::new()),
            run_name: None,
        };
        let copilot = DelayedRunner::new(
            Duration::from_millis(200),
            vec![
                AgentResult {
                    stdout: "copilot proposal".to_string(),
                    stderr: String::new(),
                    exit_code: Some(0),
                    success: true,
                },
                AgentResult {
                    stdout: "implementation summary".to_string(),
                    stderr: String::new(),
                    exit_code: Some(0),
                    success: true,
                },
            ],
        );
        let claude = DelayedRunner::new(
            Duration::from_millis(200),
            vec![
                AgentResult {
                    stdout: "claude proposal".to_string(),
                    stderr: String::new(),
                    exit_code: Some(0),
                    success: true,
                },
                AgentResult {
                    stdout: "final plan".to_string(),
                    stderr: String::new(),
                    exit_code: Some(0),
                    success: true,
                },
            ],
        );
        let pipeline = Pipeline::new(&copilot, &claude);

        let start = Instant::now();
        pipeline.execute(&cli).unwrap();

        assert!(start.elapsed() < Duration::from_millis(350));
    }

    #[test]
    fn plan_only_mode_stops_after_synthesis_and_persists_prompt() {
        let temp = tempdir().unwrap();
        let task_file = temp.path().join("task.md");
        let brainstorm = temp.path().join("prompt-brainstorm.md");
        let synthesis = temp.path().join("prompt-synthesis.md");
        let implementation = temp.path().join("prompt-implementation.md");

        fs::write(&task_file, "Build a CLI orchestrator").unwrap();
        fs::write(&brainstorm, "Brainstorm {{TASK_CONTENT}}").unwrap();
        fs::write(&synthesis, "Synthesize").unwrap();
        fs::write(&implementation, "Implement from {{PLAN_PATH}}").unwrap();

        let cli = ResolvedCli {
            task_file,
            working_dir: temp.path().to_path_buf(),
            output_root: temp.path().join("runs"),
            prompt_paths: PromptPaths {
                brainstorm,
                synthesis,
                implementation,
            },
            execution_mode: ExecutionMode::PlanOnly,
            agent_selection: crate::runner::AgentSelection::legacy_default(),
            copilot: AgentCliConfig::new("copilot".to_string(), Vec::new()),
            claude: AgentCliConfig::new("claude".to_string(), Vec::new()),
            run_name: None,
        };
        let copilot = MockRunner::new(vec![AgentResult {
            stdout: "copilot proposal".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        }]);
        let claude = MockRunner::new(vec![
            AgentResult {
                stdout: "claude proposal".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
            AgentResult {
                stdout: "final plan".to_string(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
        ]);

        let pipeline = Pipeline::new(&copilot, &claude);
        let summary = pipeline.execute(&cli).unwrap();

        assert!(temp.path().join("plan.md").exists());
        assert!(temp.path().join("implementation.prompt.md").exists());
        assert!(!summary.output_dir.join("implementation-report.md").exists());
        assert_eq!(summary.execution_mode, ExecutionMode::PlanOnly);
        assert_eq!(summary.phases.len(), 3);
        assert_eq!(summary.completed_phases, 3);
        assert_eq!(summary.total_phases, 3);
    }
}
