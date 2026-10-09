//! What an agent's pane asks, read from its screen when its terminal host says it is blocked:
//! the question, and the menu under it when there is one ("❯ 1. Yes / 2. No"), with the choice
//! the cursor is on, so an answer is the arrow keys that move there and Return.

/// A question on an agent's screen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Question {
    /// The lines that ask.
    pub text: String,
    /// The menu's choices, as the screen words them, without their numbers. Empty when it asks
    /// for typed text.
    pub choices: Vec<String>,
    /// The choice the cursor is on.
    pub selected: usize,
}

/// The cursors terminal menus mark the selected choice with.
const CURSORS: [char; 6] = ['❯', '›', '>', '▸', '▶', '→'];
/// The most lines of question kept above a menu.
const QUESTION_LINES: usize = 10;

impl Question {
    /// The keys that pick choice `index`: up or down from the cursor, then Return.
    pub fn keys_for(&self, index: usize) -> Vec<String> {
        let mut keys = Vec::new();
        let (key, count) = if index >= self.selected { ("down", index - self.selected) } else { ("up", self.selected - index) };
        keys.extend(std::iter::repeat_n(key.to_string(), count));
        keys.push("enter".into());
        keys
    }

    /// The choice that lets the agent go ahead once: the first "Yes" or "Allow", never one that
    /// also stops it asking again ("Yes, and don't ask again", "Always allow").
    pub fn allow_once(&self) -> Option<usize> {
        self.choices.iter().position(|choice| {
            let choice = choice.to_lowercase();
            let affirmative = ["yes", "allow", "approve", "proceed", "continue", "trust"].iter().any(|word| choice.starts_with(word));
            let lasting = ["always", "don't ask", "dont ask", "do not ask", "session", "remember", "all "].iter().any(|word| choice.contains(word));
            affirmative && !lasting
        })
    }
}

/// Reads what `screen` asks: its last menu and the lines above it, or, with no menu, its last
/// lines.
pub(crate) fn read(screen: &str) -> Question {
    let lines: Vec<String> = screen.lines().map(clean).collect();
    let end = lines.iter().rposition(|line| !line.trim().is_empty()).map(|index| index + 1).unwrap_or(0);
    let lines = &lines[..end];
    // The menu is the run of choices around the last line a cursor marks.
    let marked = lines.iter().rposition(|line| cursor_choice(line).is_some());
    if let Some(marked) = marked {
        let indent = choice_indent(&lines[marked]);
        let is_choice = |line: &String| cursor_choice(line).is_some() || (!line.trim().is_empty() && choice_indent(line) == indent && !line.trim_start().starts_with(['─', '━']));
        let mut first = marked;
        while first > 0 && is_choice(&lines[first - 1]) && !is_hint(&lines[first - 1]) {
            first -= 1;
        }
        let mut last = marked;
        while last + 1 < lines.len() && is_choice(&lines[last + 1]) && !is_hint(&lines[last + 1]) {
            last += 1;
        }
        let choices: Vec<String> = lines[first..=last].iter().map(|line| label(line)).collect();
        if choices.len() >= 2 {
            let question = question_above(&lines[..first]);
            return Question { text: question, choices, selected: marked - first };
        }
    }
    // Numbered choices without a cursor: "1. Yes", "2. No".
    let numbered: Vec<usize> = lines.iter().enumerate().filter(|(_, line)| numbered(line).is_some()).map(|(index, _)| index).collect();
    if let (Some(&first), Some(&last)) = (numbered.first(), numbered.last()) {
        if last - first + 1 == numbered.len() && numbered.len() >= 2 && lines[last + 1..].iter().all(|line| line.trim().is_empty() || is_hint(line)) {
            let choices = numbered.iter().map(|&index| label(&lines[index])).collect();
            return Question { text: question_above(&lines[..first]), choices, selected: 0 };
        }
    }
    Question { text: question_above(lines), choices: Vec::new(), selected: 0 }
}

/// Whether the screen ends on a menu waiting for a choice: an agent may ask one while its host
/// still reports it idle (Codex's update notice, a first-run question).
pub(crate) fn asks(screen: &str) -> bool {
    let question = read(screen);
    let Some(last) = question.choices.last() else { return false };
    if question.choices.len() < 2 {
        return false;
    }
    let lines: Vec<String> = screen.lines().map(clean).filter(|line| !line.trim().is_empty() && !is_hint(line)).collect();
    lines.iter().rev().take(2).any(|line| label(line) == *last)
}

/// A pane's screen without the agent's own chrome at its foot: the input box and the status
/// lines under it, which close with a rule a few lines from the bottom.
pub(crate) fn body(screen: &str) -> String {
    let lines: Vec<&str> = screen.trim_end().lines().collect();
    let is_rule = |line: &str| {
        let trimmed = line.trim();
        trimmed.chars().count() >= 8 && trimmed.chars().all(|c| matches!(c, '─' | '━' | '═' | '-'))
    };
    let mut end = lines.len();
    // The input box is a rule, a line or two, and a rule; the status lines follow it.
    for _ in 0..2 {
        match lines[..end].iter().rposition(|line| is_rule(line)) {
            Some(rule) if end - rule <= 6 => end = rule,
            _ => break,
        }
    }
    // An input line without rules (Codex's `› `) and the footer under it.
    if let Some(input) = lines[..end].iter().rposition(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("› ") || trimmed.starts_with("❯ ") || trimmed == "›" || trimmed == "❯"
    }) {
        if end - input <= 6 {
            end = input;
        }
    }
    lines[..end].join("\n").trim_end().to_string()
}

/// A screen line without the box drawn around it.
fn clean(line: &str) -> String {
    let line = line.trim_end();
    let line = line.trim_start_matches(['│', '┃', '║']).trim_end_matches(['│', '┃', '║']);
    line.trim_end().to_string()
}

/// The choice text after a cursor at the start of the line.
fn cursor_choice(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let first = trimmed.chars().next()?;
    if !CURSORS.contains(&first) {
        return None;
    }
    let rest = trimmed[first.len_utf8()..].trim_start();
    // A prompt with nothing after it, or a shell prompt's `> `, is not a choice.
    (!rest.is_empty() && trimmed[first.len_utf8()..].starts_with(' ')).then_some(rest)
}

/// Where a choice's text starts: after the cursor, or after the spaces that stand for one.
fn choice_indent(line: &str) -> usize {
    let leading = line.chars().take_while(|c| c.is_whitespace()).count();
    match cursor_choice(line) {
        Some(rest) => line.chars().count() - rest.chars().count(),
        None => leading,
    }
}

fn numbered(line: &str) -> Option<&str> {
    let trimmed = line.trim_start().trim_start_matches(CURSORS).trim_start();
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 2 {
        return None;
    }
    let rest = &trimmed[digits..];
    rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))
}

/// A choice as the user reads it: without the cursor and its number.
fn label(line: &str) -> String {
    let text = cursor_choice(line).unwrap_or(line).trim();
    numbered(text).unwrap_or(text).trim().to_string()
}

/// A line that says how to answer rather than what is asked: "Enter to confirm · Esc to cancel".
fn is_hint(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    numbered(line).is_none() && ["to confirm", "to cancel", "to select", "to navigate", "to go back", "enter continue", "esc skip"].iter().any(|words| lower.contains(words))
}

/// The question: the last few lines with something on them, up to a rule or a blank gap.
fn question_above(lines: &[String]) -> String {
    let mut picked: Vec<&str> = Vec::new();
    let mut blanks = 0;
    for line in lines.iter().rev() {
        let trimmed = line.trim();
        if trimmed.chars().all(|c| matches!(c, '─' | '━' | '╭' | '╮' | '╰' | '╯' | '-' | '=' | ' ')) && !trimmed.is_empty() {
            break;
        }
        if trimmed.is_empty() {
            blanks += 1;
            if blanks >= 2 && !picked.is_empty() {
                break;
            }
            continue;
        }
        blanks = 0;
        picked.push(trimmed);
        if picked.len() >= QUESTION_LINES {
            break;
        }
    }
    picked.reverse();
    picked.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_codes_trust_question_reads_with_its_choices() {
        let screen = "\n────────────────────\n Accessing workspace:\n\n /tmp/repo\n\n Quick safety check: Is this a project you created or\n one you trust?\n\n Claude Code'll be able to read, edit, and execute\n files here.\n\n Security guide\n\n ❯ No, exit\n   Yes, I trust this folder\n\n Enter to confirm · Esc to cancel\n";
        let question = read(screen);
        assert_eq!(question.choices, vec!["No, exit", "Yes, I trust this folder"]);
        assert_eq!(question.selected, 0);
        assert!(question.text.ends_with("Security guide"), "{}", question.text);
        assert_eq!(question.allow_once(), Some(1));
        assert_eq!(question.keys_for(1), vec!["down", "enter"]);
    }

    #[test]
    fn a_numbered_permission_menu_allows_once_never_always() {
        let screen = "╭──────────────────────────╮\n│ Bash command             │\n│   git push origin fix    │\n│ Do you want to proceed?  │\n│ ❯ 1. Yes                 │\n│   2. Yes, and don't ask again for git push commands │\n│   3. No, and tell Claude what to do differently (esc) │\n╰──────────────────────────╯";
        let question = read(screen);
        assert_eq!(question.choices.len(), 3);
        assert_eq!(question.choices[0], "Yes");
        assert!(question.text.contains("git push origin fix") && question.text.contains("Do you want to proceed?"));
        assert_eq!(question.allow_once(), Some(0));
        assert_eq!(question.keys_for(0), vec!["enter"]);
        assert_eq!(Question { selected: 2, ..question.clone() }.keys_for(0), vec!["up", "up", "enter"]);
        let always = Question { text: String::new(), choices: vec!["Always allow".into(), "No".into()], selected: 0 };
        assert_eq!(always.allow_once(), None);
    }

    #[test]
    fn a_menu_at_the_foot_of_the_screen_asks_and_an_input_box_does_not() {
        let update = "  Update available · 0.161.0 → 0.162.1\n\n› 1. Update now (runs `npm install -g @openai/codex`)\n  2. Skip\n  3. Skip until next version\n\n  enter continue · esc skip\n";
        assert!(asks(update));
        let idle = "⏺ Done.\n\n──────────────\n❯ Try \"fix typecheck errors\"\n──────────────\n  ⏵⏵ auto mode on (shift+tab to cycle)\n";
        assert!(!asks(idle));
        assert!(!asks("1. First step\n2. Second step\n\nAll done, next steps above."));
    }

    #[test]
    fn the_input_box_and_status_lines_are_not_the_body() {
        let screen = "⏺ Done: committed NOTES.md.\n\n✻ Baked for 12s\n\n──────────────\n❯ \n──────────────\n  ⏵⏵ auto mode on (shift+tab to cycle)\n";
        assert_eq!(body(screen), "⏺ Done: committed NOTES.md.\n\n✻ Baked for 12s");
        assert_eq!(body("one\ntwo"), "one\ntwo");
        let codex = "• Committed NOTES.md.\n\n  Worked for 17s\n\n\n› Ask Codex to do anything\n\n  GPT-6.1 high · /tmp/x\n  ? for shortcuts\n";
        assert_eq!(body(codex), "• Committed NOTES.md.\n\n  Worked for 17s");
    }

    #[test]
    fn a_question_without_a_menu_is_its_last_lines() {
        let question = read("Building…\n\nWhich port should the server use?\n> \n");
        assert!(question.choices.is_empty());
        assert!(question.text.ends_with("Which port should the server use?\n>"), "{}", question.text);
    }
}
