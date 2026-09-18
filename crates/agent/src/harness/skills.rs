//! Skills: `SKILL.md` files (agentskills.io) the model is told about and reads when a task
//! matches, after pi's skill loader.

use std::path::{Path, PathBuf};

use super::frontmatter::parse_frontmatter;

const MAX_NAME_LENGTH: usize = 64;
const MAX_DESCRIPTION_LENGTH: usize = 1024;

/// A skill loaded from a `SKILL.md` file or given by the application. `name`, `description`,
/// and `path` go into the system prompt; `content` is what the model reads on invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    /// The file's absolute path, for the model's listing and relative references.
    pub path: PathBuf,
    /// Left out of the model's listing; the application can still invoke it.
    pub disable_model_invocation: bool,
}

/// A problem found while loading, with the file it concerns. Loading goes on regardless.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDiagnostic {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LoadedSkills {
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

/// Loads skills from directories: every `SKILL.md` below them (the first one in a directory
/// wins and its subdirectories are not searched), plus `.md` files directly in a root that
/// carry a `description` (named after the file). Ignore files are honored; a missing directory
/// is skipped.
pub fn load_skills(dirs: &[PathBuf]) -> LoadedSkills {
    let mut loaded = LoadedSkills::default();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let mut walker = ignore::WalkBuilder::new(dir);
        walker.hidden(true).git_ignore(true).ignore(true).filter_entry(|entry| entry.file_name() != "node_modules");
        let mut claimed: Vec<PathBuf> = Vec::new();
        let mut entries: Vec<PathBuf> = walker.build().filter_map(Result::ok).filter(|e| e.file_type().is_some_and(|t| t.is_file())).map(|e| e.into_path()).collect();
        entries.sort();
        // A SKILL.md claims its directory: nothing below it is another skill.
        let skill_files: Vec<PathBuf> = entries.iter().filter(|p| p.file_name().is_some_and(|n| n == "SKILL.md")).cloned().collect();
        for file in skill_files {
            let parent = file.parent().map(Path::to_path_buf).unwrap_or_default();
            if claimed.iter().any(|c| parent.starts_with(c) && parent != *c) {
                continue;
            }
            claimed.push(parent.clone());
            load_file(&file, true, &mut loaded);
        }
        for file in entries.iter().filter(|p| p.parent() == Some(dir.as_path()) && p.extension().is_some_and(|e| e == "md") && p.file_name().is_some_and(|n| n != "SKILL.md")) {
            load_file(file, false, &mut loaded);
        }
    }
    loaded
}

fn load_file(path: &Path, declared: bool, into: &mut LoadedSkills) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            into.diagnostics.push(SkillDiagnostic { path: path.to_path_buf(), message: format!("cannot read: {error}") });
            return;
        }
    };
    let parsed = parse_frontmatter(&text);
    let description = parsed.get("description").map(str::trim).unwrap_or("");
    if description.is_empty() {
        if declared {
            into.diagnostics.push(SkillDiagnostic { path: path.to_path_buf(), message: "description is required".into() });
        }
        return;
    }
    if description.len() > MAX_DESCRIPTION_LENGTH {
        into.diagnostics.push(SkillDiagnostic { path: path.to_path_buf(), message: format!("description exceeds {MAX_DESCRIPTION_LENGTH} characters") });
    }
    let parent_name = path.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    // A declared skill is named after its directory; a root `.md` file after itself.
    let fallback = if declared { parent_name.clone() } else { path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default() };
    let name = parsed.get("name").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or(fallback);
    for problem in validate_name(&name, &parent_name, declared) {
        into.diagnostics.push(SkillDiagnostic { path: path.to_path_buf(), message: problem });
    }
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    into.skills.push(Skill {
        name,
        description: description.to_string(),
        content: parsed.body.clone(),
        path: absolute,
        disable_model_invocation: parsed.flag("disable-model-invocation"),
    });
}

fn validate_name(name: &str, parent: &str, declared: bool) -> Vec<String> {
    let mut problems = Vec::new();
    if declared && name != parent {
        problems.push(format!("name \"{name}\" does not match parent directory \"{parent}\""));
    }
    if name.len() > MAX_NAME_LENGTH {
        problems.push(format!("name exceeds {MAX_NAME_LENGTH} characters"));
    }
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        problems.push("name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)".into());
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        problems.push("name must not start or end with a hyphen or contain consecutive hyphens".into());
    }
    problems
}

/// The prompt that invokes a skill: its content in a `<skill>` block, plus what the user added.
pub fn format_skill_invocation(skill: &Skill, additional_instructions: Option<&str>) -> String {
    let dir = skill.path.parent().map(|p| p.display().to_string()).unwrap_or_else(|| "/".into());
    let block = format!("<skill name=\"{}\" location=\"{}\">\nReferences are relative to {dir}.\n\n{}\n</skill>", skill.name, skill.path.display(), skill.content);
    match additional_instructions.map(str::trim).filter(|s| !s.is_empty()) {
        Some(extra) => format!("{block}\n\n{extra}"),
        None => block,
    }
}

/// The system prompt block listing the skills the model may read, in agentskills.io's shape.
/// Empty when no skill is model-visible.
pub fn format_skills_for_system_prompt(skills: &[Skill]) -> String {
    let visible: Vec<&Skill> = skills.iter().filter(|s| !s.disable_model_invocation).collect();
    if visible.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        "The following skills provide specialized instructions for specific tasks.".to_string(),
        "Read the full skill file when the task matches its description.".to_string(),
        "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".to_string(),
        String::new(),
        "<available_skills>".to_string(),
    ];
    for skill in visible {
        lines.push("  <skill>".into());
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!("    <description>{}</description>", escape_xml(&skill.description)));
        lines.push(format!("    <location>{}</location>", escape_xml(&skill.path.display().to_string())));
        lines.push("  </skill>".into());
    }
    lines.push("</available_skills>".into());
    lines.join("\n")
}

fn escape_xml(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_declared_skills_and_root_markdown_and_reports_problems() {
        let root = std::env::temp_dir().join(format!("lorca-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("deploy/nested")).unwrap();
        std::fs::create_dir_all(root.join("Bad Name")).unwrap();
        std::fs::write(root.join("deploy/SKILL.md"), "---\nname: deploy\ndescription: Ship a release\n---\nSteps here.").unwrap();
        std::fs::write(root.join("deploy/nested/SKILL.md"), "---\nname: nested\ndescription: hidden below a skill\n---\nx").unwrap();
        std::fs::write(root.join("Bad Name/SKILL.md"), "---\ndescription: no name field\n---\nx").unwrap();
        std::fs::write(root.join("notes.md"), "---\ndescription: A root note skill\ndisable-model-invocation: true\n---\nnote body").unwrap();
        std::fs::write(root.join("plain.md"), "no frontmatter").unwrap();
        let loaded = load_skills(&[root.clone(), root.join("missing")]);
        let names: Vec<&str> = loaded.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Bad Name", "deploy", "notes"]);
        assert!(loaded.skills.iter().any(|s| s.name == "notes" && s.disable_model_invocation && s.content == "note body"));
        assert!(loaded.diagnostics.iter().any(|d| d.message.contains("invalid characters")));
        let prompt = format_skills_for_system_prompt(&loaded.skills);
        assert!(prompt.contains("<name>deploy</name>") && prompt.contains("Ship a release") && !prompt.contains("<name>notes</name>"));
        let invocation = format_skill_invocation(&loaded.skills[1], Some("focus on staging"));
        assert!(invocation.starts_with("<skill name=\"deploy\"") && invocation.ends_with("focus on staging"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
