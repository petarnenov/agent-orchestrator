use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::cli::PromptPaths;
use crate::runner::Phase;

pub const BUILTIN_BRAINSTORM_PROMPT: &str = include_str!("../prompt-brainstorm.md");
pub const BUILTIN_SYNTHESIS_PROMPT: &str = include_str!("../prompt-synthesis.md");
pub const BUILTIN_IMPLEMENTATION_PROMPT: &str = include_str!("../prompt-implementation.md");

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
    pub prospect1_path: PathBuf,
    pub prospect2_path: PathBuf,
    pub plan_path: PathBuf,
}

impl PromptTemplates {
    pub fn bundled() -> Self {
        Self {
            brainstorm: BUILTIN_BRAINSTORM_PROMPT.to_string(),
            synthesis: BUILTIN_SYNTHESIS_PROMPT.to_string(),
            implementation: BUILTIN_IMPLEMENTATION_PROMPT.to_string(),
        }
    }

    pub fn load(paths: &PromptPaths) -> Result<Self> {
        let bundled = Self::bundled();

        Ok(Self {
            brainstorm: load_prompt(paths.brainstorm.as_deref(), &bundled.brainstorm)?,
            synthesis: load_prompt(paths.synthesis.as_deref(), &bundled.synthesis)?,
            implementation: load_prompt(paths.implementation.as_deref(), &bundled.implementation)?,
        })
    }
}

fn load_prompt(path: Option<&Path>, bundled: &str) -> Result<String> {
    match path {
        Some(path) => {
            fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
        }
        None => Ok(bundled.to_string()),
    }
}

pub fn render_prompt(phase: Phase, templates: &PromptTemplates, context: &PromptContext) -> String {
    let template = match phase {
        Phase::Prospect1 | Phase::Prospect2 => &templates.brainstorm,
        Phase::Synthesis => &templates.synthesis,
        Phase::Implementation => &templates.implementation,
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
        .replace("{{PROSPECT1_PATH}}", &display_path(&context.prospect1_path))
        .replace("{{PROSPECT2_PATH}}", &display_path(&context.prospect2_path))
        .replace(
            "{{COPILOT_PROPOSAL_PATH}}",
            &display_path(&context.prospect1_path),
        )
        .replace(
            "{{CLAUDE_PROPOSAL_PATH}}",
            &display_path(&context.prospect2_path),
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
        Phase::Prospect1 | Phase::Prospect2 => {
            lines.push(format!(
                "Your response will be captured and saved to: {}",
                display_path(&context.target_output_path)
            ));
        }
        Phase::Synthesis => {
            lines.push(format!(
                "Prospect1 path: {}",
                display_path(&context.prospect1_path)
            ));
            lines.push(format!(
                "Prospect2 path: {}",
                display_path(&context.prospect2_path)
            ));
            lines.push(format!(
                "Write only the final consolidated plan. The orchestrator will save your response to: {}",
                display_path(&context.target_output_path)
            ));
        }
        Phase::Implementation => {
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
    use tempfile::tempdir;

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
            prospect1_path: PathBuf::from("/tmp/prospect1.md"),
            prospect2_path: PathBuf::from("/tmp/prospect2.md"),
            plan_path: PathBuf::from("/tmp/plan.md"),
        };

        let rendered = render_prompt(Phase::Prospect1, &templates, &context);

        assert!(rendered.contains("/tmp/task.md"));
        assert!(rendered.contains("Build something"));
        assert!(rendered.contains("/tmp/out.md"));
        assert!(rendered.contains("# Runtime Context"));
    }

    #[test]
    fn loads_bundled_prompts_when_paths_are_missing() {
        let templates = PromptTemplates::load(&PromptPaths {
            brainstorm: None,
            synthesis: None,
            implementation: None,
        })
        .unwrap();

        assert_eq!(templates.brainstorm, BUILTIN_BRAINSTORM_PROMPT);
        assert_eq!(templates.synthesis, BUILTIN_SYNTHESIS_PROMPT);
        assert_eq!(templates.implementation, BUILTIN_IMPLEMENTATION_PROMPT);
    }

    #[test]
    fn prefers_explicit_prompt_files_over_bundled_defaults() {
        let temp = tempdir().unwrap();
        let brainstorm = temp.path().join("brainstorm.md");
        let synthesis = temp.path().join("synthesis.md");
        let implementation = temp.path().join("implementation.md");
        fs::write(&brainstorm, "custom brainstorm").unwrap();
        fs::write(&synthesis, "custom synthesis").unwrap();
        fs::write(&implementation, "custom implementation").unwrap();

        let templates = PromptTemplates::load(&PromptPaths {
            brainstorm: Some(brainstorm),
            synthesis: Some(synthesis),
            implementation: Some(implementation),
        })
        .unwrap();

        assert_eq!(templates.brainstorm, "custom brainstorm");
        assert_eq!(templates.synthesis, "custom synthesis");
        assert_eq!(templates.implementation, "custom implementation");
    }
}
