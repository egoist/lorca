//! The system prompt a general coding agent runs with: who it is, how to work, its tools,
//! its skills, and where it is. Any part can be replaced.

use std::path::Path;

use super::skills::{format_skills_for_system_prompt, Skill};
use crate::tools::{coding_tools_guidelines, coding_tools_snippet};

/// What goes into the default prompt.
#[derive(Debug, Clone, Default)]
pub struct SystemPromptParts<'a> {
    /// The opening: who the agent is and what it is for. `None` is a coding assistant.
    pub identity: Option<&'a str>,
    /// The working directory, named so relative paths mean something.
    pub cwd: Option<&'a Path>,
    /// Extra guidance from the application or the user (project instructions, preferences).
    pub instructions: &'a [&'a str],
    /// Whether the built-in coding tools are available, so their guidelines are included.
    pub coding_tools: bool,
    pub skills: &'a [Skill],
    /// The date the prompt is written for, so the model knows how old its knowledge is.
    pub date: Option<&'a str>,
}

/// The prompt, in the order the model reads best: identity, guidelines, tools, skills, place.
pub fn build_system_prompt(parts: &SystemPromptParts<'_>) -> String {
    let mut prompt = String::new();
    prompt.push_str(parts.identity.unwrap_or("You are a coding assistant that works in the user's project with the tools you are given."));
    prompt.push_str("\n\n");
    prompt.push_str(
        "Work carefully: read before you edit, keep changes small and complete, and say what you did and what you did not verify. \
         Prefer the tools over guessing. When a task is ambiguous in a way that changes the work, ask; otherwise decide and note the assumption.",
    );
    if parts.coding_tools {
        prompt.push_str(&format!("\n\nTools: {}\n", coding_tools_snippet()));
        for guideline in coding_tools_guidelines() {
            prompt.push_str(&format!("- {guideline}\n"));
        }
    }
    for instruction in parts.instructions {
        if !instruction.trim().is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(instruction.trim());
        }
    }
    let skills = format_skills_for_system_prompt(parts.skills);
    if !skills.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(&skills);
    }
    if let Some(cwd) = parts.cwd {
        prompt.push_str(&format!("\n\nThe working directory is {}. Relative paths resolve there.", cwd.display()));
    }
    if let Some(date) = parts.date {
        prompt.push_str(&format!("\nToday is {date}."));
    }
    prompt.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_names_tools_skills_and_place() {
        let skill = Skill { name: "deploy".into(), description: "Ship it".into(), content: String::new(), path: "/s/deploy/SKILL.md".into(), disable_model_invocation: false };
        let prompt = build_system_prompt(&SystemPromptParts { identity: None, cwd: Some(Path::new("/work")), instructions: &["Use tabs."], coding_tools: true, skills: std::slice::from_ref(&skill), date: Some("2026-09-16") });
        assert!(prompt.starts_with("You are a coding assistant"));
        assert!(prompt.contains("Tools: ") && prompt.contains("Use tabs.") && prompt.contains("<name>deploy</name>"));
        assert!(prompt.ends_with("The working directory is /work. Relative paths resolve there.\nToday is 2026-09-16."));
        assert!(!build_system_prompt(&SystemPromptParts::default()).contains("Tools:"));
    }
}
