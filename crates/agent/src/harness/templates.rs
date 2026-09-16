//! Prompt templates: Markdown files a user invokes by name with arguments, after pi's prompt
//! templates. `$1`, `$2`, `$@`, `$ARGUMENTS`, `${@:N}`, and `${@:N:L}` are filled in.

use std::path::{Path, PathBuf};

use super::frontmatter::parse_frontmatter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate {
    /// The file name without `.md`.
    pub name: String,
    /// From the frontmatter, else the body's first line.
    pub description: String,
    /// A hint for the arguments, from the frontmatter.
    pub argument_hint: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateDiagnostic {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LoadedTemplates {
    pub templates: Vec<PromptTemplate>,
    pub diagnostics: Vec<TemplateDiagnostic>,
}

/// Loads templates: the `.md` files directly in each directory given, and each `.md` file
/// given. Missing paths are skipped.
pub fn load_prompt_templates(paths: &[PathBuf]) -> LoadedTemplates {
    let mut loaded = LoadedTemplates::default();
    for path in paths {
        if path.is_dir() {
            let mut files: Vec<PathBuf> = match std::fs::read_dir(path) {
                Ok(entries) => entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "md")).collect(),
                Err(error) => {
                    loaded.diagnostics.push(TemplateDiagnostic { path: path.clone(), message: format!("cannot list: {error}") });
                    continue;
                }
            };
            files.sort();
            for file in files {
                load_file(&file, &mut loaded);
            }
        } else if path.is_file() && path.extension().is_some_and(|e| e == "md") {
            load_file(path, &mut loaded);
        }
    }
    loaded
}

fn load_file(path: &Path, into: &mut LoadedTemplates) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            into.diagnostics.push(TemplateDiagnostic { path: path.to_path_buf(), message: format!("cannot read: {error}") });
            return;
        }
    };
    let parsed = parse_frontmatter(&text);
    let description = match parsed.get("description") {
        Some(description) => description.trim().to_string(),
        None => {
            let first = parsed.body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            if first.chars().count() > 60 { format!("{}...", first.chars().take(60).collect::<String>()) } else { first.to_string() }
        }
    };
    into.templates.push(PromptTemplate {
        name: path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        description,
        argument_hint: parsed.get("argument-hint").map(str::to_string),
        content: parsed.body.clone(),
    });
}

/// Splits an argument string the way a shell would, with single and double quotes.
pub fn parse_command_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in input.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == ' ' || c == '\t' => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            None => current.push(c),
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

/// Fills the placeholders of a template with `args`.
pub fn substitute_args(content: &str, args: &[String]) -> String {
    let all = args.join(" ");
    let mut out = String::with_capacity(content.len());
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let rest: String = chars[i + 1..].iter().collect();
        if let Some(stripped) = rest.strip_prefix("ARGUMENTS") {
            let _ = stripped;
            out.push_str(&all);
            i += 1 + "ARGUMENTS".len();
        } else if rest.starts_with('@') {
            out.push_str(&all);
            i += 2;
        } else if rest.starts_with("{@:") {
            let Some(close) = rest.find('}') else {
                out.push('$');
                i += 1;
                continue;
            };
            let spec = &rest[3..close];
            let (start, length) = match spec.split_once(':') {
                Some((s, l)) => (s.parse::<usize>().ok(), l.parse::<usize>().ok()),
                None => (spec.parse::<usize>().ok(), None),
            };
            let start = start.unwrap_or(1).saturating_sub(1).min(args.len());
            let slice = match length {
                Some(len) => &args[start..(start + len).min(args.len())],
                None => &args[start..],
            };
            out.push_str(&slice.join(" "));
            i += 1 + close + 1;
        } else {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if digits.is_empty() {
                out.push('$');
                i += 1;
            } else {
                let index: usize = digits.parse().unwrap_or(0);
                if index >= 1 {
                    out.push_str(args.get(index - 1).map(String::as_str).unwrap_or(""));
                }
                i += 1 + digits.len();
            }
        }
    }
    out
}

/// The prompt a template makes with `args`.
pub fn format_prompt_template_invocation(template: &PromptTemplate, args: &[String]) -> String {
    substitute_args(&template.content, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_positional_and_rest_arguments() {
        let args = parse_command_args("one \"two words\" 'three' four");
        assert_eq!(args, vec!["one", "two words", "three", "four"]);
        assert_eq!(substitute_args("first=$1 second=$2 all=$@ rest=${@:2} two=${@:2:2} missing=$9 $ARGUMENTS $x", &args), "first=one second=two words all=one two words three four rest=two words three four two=two words three missing= one two words three four $x");
    }

    #[test]
    fn loads_templates_with_or_without_frontmatter() {
        let root = std::env::temp_dir().join(format!("tinybot-templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("review.md"), "---\ndescription: Review a diff\nargument-hint: <path>\n---\nReview $1 carefully.").unwrap();
        std::fs::write(root.join("fix.md"), "Fix the failing test named $1 and explain what was wrong in one line.\nMore.").unwrap();
        let loaded = load_prompt_templates(std::slice::from_ref(&root));
        let names: Vec<&str> = loaded.templates.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["fix", "review"]);
        assert_eq!(loaded.templates[1].description, "Review a diff");
        assert_eq!(loaded.templates[1].argument_hint.as_deref(), Some("<path>"));
        assert_eq!(loaded.templates[0].description, format!("{}...", "Fix the failing test named $1 and explain what was wrong in one line.".chars().take(60).collect::<String>()));
        assert_eq!(format_prompt_template_invocation(&loaded.templates[1], &["src/a.rs".into()]), "Review src/a.rs carefully.");
        let _ = std::fs::remove_dir_all(&root);
    }
}
