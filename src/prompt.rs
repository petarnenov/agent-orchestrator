use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::cli::PromptPaths;
use crate::runner::Phase;

#[derive(Debug, Clone)]
pub struct PromptTemplates {
    pub brainstorm: String,
    pub synthesis: String,
    pub implementation: String,
}

#[derive(Debug, Clone)]
pub struct PromptContext {
    pub task_file_path: PathBuf,
    pub task_content: String,
    pub workspace_dir: PathBuf,
    pub target_output_path: PathBuf,
    pub copilot_proposal_path: PathBuf,
    pub claude_proposal_path: PathBuf,
    pub plan_path: PathBuf,
}

impl PromptTemplates {
    pub fn load(paths: &PromptPaths) -> Result<Self> {
        Ok(Self {
            brainstorm: fs::read_to_string(&paths.brainstorm)
                .with_context(|| format!("failed to read {}", paths.brainstorm.display()))?,
            synthesis: fs::read_to_string(&paths.synthesis)
                .with_context(|| format!("failed to read {}", paths.synthesis.display()))?,
            implementation: fs::read_to_string(&paths.implementation)
                .with_context(|| format!("failed to read {}", paths.implementation.display()))?,
        })
    }
}

pub fn render_prompt(phase: Phase, templates: &PromptTemplates, context: &PromptContext) -> String {
    let template = match phase {
        Phase::BrainstormCopilot | Phase::BrainstormClaude => &templates.brainstorm,
        Phase::SynthesisClaude => &templates.synthesis,
        Phase::ImplementationCopilot => &templates.implementation,
    };

    let with_placeholders = replace_placeholders(template, context);
    format!("{with_placeholders}\n\n{}", runtime_block(phase, context))
}

fn replace_placeholders(template: &str, context: &PromptContext) -> String {
    template
        .replace("{{TASK_FILE_PATH}}", &display_path(&context.task_file_path))
        .replace("{{TASK_CONTENT}}", &context.task_content)
        .replace("{{WORKSPACE_DIR}}", &display_path(&context.workspace_dir))
        .replace(
            "{{TARGET_OUTPUT_PATH}}",
            &display_path(&context.target_output_path),
        )
        .replace(
            "{{COPILOT_PROPOSAL_PATH}}",
            &display_path(&context.copilot_proposal_path),
        )
        .replace(
            "{{CLAUDE_PROPOSAL_PATH}}",
            &display_path(&context.claude_proposal_path),
        )
        .replace("{{PLAN_PATH}}", &display_path(&context.plan_path))
}

fn runtime_block(phase: Phase, context: &PromptContext) -> String {
    let mut lines = vec![
        "# Runtime Context".to_string(),
        format!("Task file: {}", display_path(&context.task_file_path)),
        format!("Workspace root: {}", display_path(&context.workspace_dir)),
        String::new(),
        "Task:".to_string(),
        context.task_content.clone(),
        String::new(),
    ];

    match phase {
        Phase::BrainstormCopilot | Phase::BrainstormClaude => {
            lines.push(format!(
                "Your response will be captured and saved to: {}",
                display_path(&context.target_output_path)
            ));
        }
        Phase::SynthesisClaude => {
            lines.push(format!(
                "Copilot proposal path: {}",
                display_path(&context.copilot_proposal_path)
            ));
            lines.push(format!(
                "Claude proposal path: {}",
                display_path(&context.claude_proposal_path)
            ));
            lines.push(format!(
                "Write only the final consolidated plan. The orchestrator will save your response to: {}",
                display_path(&context.target_output_path)
            ));
        }
        Phase::ImplementationCopilot => {
            lines.push(format!(
                "Approved plan path: {}",
                display_path(&context.plan_path)
            ));
            lines.push(format!(
                "After implementation, provide the final summary only. The orchestrator will save your response to: {}",
                display_path(&context.target_output_path)
            ));
        }
    }

    lines.join("\n")
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_placeholders_and_runtime_context() {
        let templates = PromptTemplates {
            brainstorm: "Task file {{TASK_FILE_PATH}}\n{{TASK_CONTENT}}\n-> {{TARGET_OUTPUT_PATH}}"
                .to_string(),
            synthesis: String::new(),
            implementation: String::new(),
        };
        let context = PromptContext {
            task_file_path: PathBuf::from("/tmp/task.md"),
            task_content: "Build something".to_string(),
            workspace_dir: PathBuf::from("/tmp/workspace"),
            target_output_path: PathBuf::from("/tmp/out.md"),
            copilot_proposal_path: PathBuf::from("/tmp/copilot.md"),
            claude_proposal_path: PathBuf::from("/tmp/claude.md"),
            plan_path: PathBuf::from("/tmp/plan.md"),
        };

        let rendered = render_prompt(Phase::BrainstormCopilot, &templates, &context);

        assert!(rendered.contains("/tmp/task.md"));
        assert!(rendered.contains("Build something"));
        assert!(rendered.contains("/tmp/out.md"));
        assert!(rendered.contains("# Runtime Context"));
    }
}
