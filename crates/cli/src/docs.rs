//! The builtin `lorca` skill every bot's skill catalog lists: where Lorca's user docs are on
//! lorca.app, as Markdown, so a bot answers how to use Lorca from them instead of guessing, or
//! reading the CLI's binary for strings.

use serde_json::{json, Value};

/// The skill's name, and its directory under `playbook://`, which no saved skill's id
/// (`playbook-…`) can take.
pub const NAME: &str = "lorca";

const DESCRIPTION: &str = "Lorca's user docs: what it can do and how to use it.";

/// The skill's row in the turn's catalog, as `playbooks::catalog` writes the user's.
pub fn catalog_row() -> Value {
    json!({ "name": NAME, "description": DESCRIPTION, "scope": "builtin", "path": format!("playbook://{NAME}/SKILL.md") })
}

/// Whether a `list_playbooks` query finds the skill, as it finds the user's.
pub fn matches(query: &str) -> bool {
    format!("{NAME} {DESCRIPTION}").to_lowercase().contains(&query.to_lowercase())
}

/// A file of the skill by its `playbook://lorca/` path; None for another skill's path.
pub fn read(path: &str) -> Option<Result<String, String>> {
    let resource = path.strip_prefix("playbook://")?.strip_prefix(NAME)?.strip_prefix('/')?;
    Some(if resource == "SKILL.md" { Ok(skill()) } else { Err(format!("The {NAME} skill has only SKILL.md.")) })
}

fn skill() -> String {
    format!(
        "---\nname: {NAME}\ndescription: {DESCRIPTION}\n---\n\n\
         Lorca's user docs are on its website, as Markdown:\n\n\
         - https://lorca.app/llms.txt lists every page by section, with a line on what each covers. Start here.\n\
         - https://lorca.app/docs/<page>.md is one page, such as https://lorca.app/docs/routines.md. A link in the list or \
         in a page, such as /docs/routines#timezones, names that page.\n\
         - https://lorca.app/llms-full.txt is every page in one file, to search across them.\n\
         - The Simplified Chinese pages are under /zh: https://lorca.app/zh/llms.txt and https://lorca.app/zh/docs/<page>.md.\n\n\
         When the user asks what Lorca can do, how to do something in it, or why it behaves as it does, read the list, then \
         the pages that cover it, and answer from them, with the steps and the names of the menus and settings they give. \
         Fetch them with your web fetch tool when you have one, else with `curl -fsSL <url>`. Do not answer from memory, and \
         do not read Lorca's binary, app bundle, or data files to find out how it works.\n\n\
         The website describes the newest Lorca. When the user's app lacks something a page describes, an update brings it.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skill_points_at_the_website() {
        let skill = read("playbook://lorca/SKILL.md").unwrap().unwrap();
        assert!(skill.starts_with("---\nname: lorca\ndescription: Lorca's user docs"));
        assert!(skill.contains("https://lorca.app/llms.txt") && skill.contains("https://lorca.app/docs/<page>.md"));
        assert!(read("playbook://lorca/references/routines.md").unwrap().is_err());
        assert!(read("playbook://playbook-123/SKILL.md").is_none());
        assert!(matches("DOCS") && matches("") && !matches("zzqx"));
        assert_eq!(catalog_row()["path"], "playbook://lorca/SKILL.md");
    }
}
