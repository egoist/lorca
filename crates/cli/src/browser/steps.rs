//! A recorded browser workflow as a skill keeps it: `scripts/browser-steps.json`, the steps
//! `browser_session { action: "run" }` repeats in one of the bot's profiles. Each step names the
//! element it acts on by Playwright locators, the first that matches the page wins, and may say
//! what the page shows afterwards. `{{name}}` stands for an input the run is given, and
//! `{{secret:NAME}}`, only in what a `fill` step types, for one of the bot's saved secrets
//! (`crate::secrets`), which the run fills in on the secret's site.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Where a skill keeps its steps.
pub const PATH: &str = "scripts/browser-steps.json";
pub const MAX_STEPS: usize = 100;
const MAX_TARGETS: usize = 8;
const MAX_VALUE: usize = 2000;
/// How long a step waits for its element, and for what it expects afterwards, by default.
pub const DEFAULT_TIMEOUT_SECS: u64 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Goto,
    Click,
    Fill,
    Select,
    Check,
    Uncheck,
    Press,
    Upload,
}

/// What the page shows once the step is done: its address or title contains the text, or the
/// text is on the page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub action: Action,
    /// The element in words, for people and for the message when the step stops a run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    /// What the step is for, in the skill author's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// A password or a field marked secret, which the recording does not keep.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<Expect>,
    /// Seconds the step waits for its element and its `expect`, 1 to 60.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Steps {
    /// The bot's browser profile the steps run in, by name.
    pub profile: String,
    /// The values a run is given, by name, with what each one is.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, String>,
    pub steps: Vec<Step>,
}

fn input_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.starts_with(|c: char| c.is_ascii_lowercase()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The text with each `{{name}}` it has in `inputs` replaced by its value.
fn fill(text: &str, inputs: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        out.push_str(&rest[..start]);
        match inputs.get(after[..end].trim()) {
            Some(value) => out.push_str(value),
            None => out.push_str(&rest[start..start + end + 4]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// The saved secret a `{{secret:NAME}}` names: NAME, when the placeholder is one.
fn secret_name(placeholder: &str) -> Option<&str> {
    placeholder.strip_prefix("secret:").map(str::trim)
}

/// The saved secrets the steps type, by name.
pub fn secrets_named(steps: &Steps) -> Vec<String> {
    let mut names: Vec<String> = steps.steps.iter().flat_map(|step| step.value.iter()).flat_map(|value| placeholders(value)).filter_map(secret_name).map(str::to_string).collect();
    names.sort();
    names.dedup();
    names
}

/// The `{{name}}`s in a text, `{{secret:NAME}}`s included.
fn placeholders(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        names.push(after[..end].trim());
        rest = &after[end + 2..];
    }
    names
}

fn short(text: &str, max: usize, what: &str) -> Result<(), String> {
    if text.chars().count() > max || text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err(format!("{what} is too long or has control characters"));
    }
    Ok(())
}

impl Step {
    /// Every text the step fills in at run time.
    fn texts(&self) -> impl Iterator<Item = &String> {
        self.url.iter().chain(&self.value).chain(&self.values).chain(&self.files).chain(self.expect.iter().flat_map(|e| e.url.iter().chain(&e.title).chain(&e.text)))
    }

    /// The step in a few words: "click “Create” button", for Auto-review and the run's messages.
    pub fn describe(&self) -> String {
        let element = self.element.clone().or_else(|| self.targets.first().cloned()).unwrap_or_default();
        match self.action {
            Action::Goto => format!("open {}", self.url.as_deref().unwrap_or_default()),
            Action::Click => format!("click {element}"),
            Action::Fill if self.secret => format!("type a password into {element}"),
            Action::Fill => format!("type “{}” into {element}", self.value.as_deref().unwrap_or_default().chars().take(80).collect::<String>()),
            Action::Select => format!("choose {} in {element}", self.values.join(", ")),
            Action::Check => format!("check {element}"),
            Action::Uncheck => format!("uncheck {element}"),
            Action::Press => format!("press {}", self.key.as_deref().unwrap_or_default()),
            Action::Upload => format!("upload {} with {element}", self.files.join(", ")),
        }
    }
}

/// Reads and checks a steps file: every step has what its action needs, within bounds, and
/// every `{{name}}` is one of `inputs`.
pub fn parse(text: &str) -> Result<Steps, String> {
    let steps: Steps = serde_json::from_str(text).map_err(|e| format!("{PATH}: {e}"))?;
    let fail = |index: usize, why: &str| Err(format!("{PATH}: step {} {why}", index + 1));
    if steps.profile.trim().is_empty() || steps.profile.chars().count() > 100 {
        return Err(format!("{PATH}: name the browser profile the steps run in"));
    }
    if steps.steps.is_empty() || steps.steps.len() > MAX_STEPS {
        return Err(format!("{PATH}: have 1 to {MAX_STEPS} steps"));
    }
    for (name, about) in &steps.inputs {
        if !input_name(name) {
            return Err(format!("{PATH}: input names are lowercase letters, numbers, and underscores: {name}"));
        }
        short(about, 300, &format!("{PATH}: input {name}"))?;
    }
    for (index, step) in steps.steps.iter().enumerate() {
        let needs_targets = !matches!(step.action, Action::Goto | Action::Press);
        if needs_targets && (step.targets.is_empty() || step.targets.len() > MAX_TARGETS) {
            return fail(index, &format!("needs 1 to {MAX_TARGETS} targets"));
        }
        if !needs_targets && !step.targets.is_empty() {
            return fail(index, "takes no targets");
        }
        if step.targets.iter().any(|target| target.trim().is_empty() || target.chars().count() > 600 || target.contains('\n')) {
            return fail(index, "has a target that is empty, too long, or on several lines");
        }
        let ok = match step.action {
            Action::Goto => step.url.as_deref().is_some_and(|url| url.starts_with("https://") || url.starts_with("http://") || url.starts_with("{{")),
            Action::Fill => step.secret != step.value.is_some(),
            Action::Select => !step.values.is_empty() && step.values.len() <= 20,
            Action::Press => step.key.as_deref().is_some_and(|key| !key.is_empty() && key.len() <= 30 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+')),
            Action::Upload => !step.files.is_empty() && step.files.len() <= 20,
            Action::Click | Action::Check | Action::Uncheck => true,
        };
        if !ok {
            return fail(index, match step.action {
                Action::Goto => "needs an http or https url",
                Action::Fill => "needs a value, or secret: true",
                Action::Select => "needs 1 to 20 values",
                Action::Press => "needs a key such as Enter",
                _ => "needs 1 to 20 files",
            });
        }
        if let Some(expect) = &step.expect {
            if expect.url.is_none() && expect.title.is_none() && expect.text.is_none() {
                return fail(index, "expects nothing: give url, title, or text");
            }
        }
        if step.timeout.is_some_and(|secs| !(1..=60).contains(&secs)) {
            return fail(index, "waits 1 to 60 seconds");
        }
        for text in step.texts() {
            short(text, MAX_VALUE, &format!("{PATH}: step {}", index + 1))?;
            for name in placeholders(text) {
                match secret_name(name) {
                    // A saved secret goes only into what Browser types.
                    Some(secret) if step.action != Action::Fill || step.value.as_ref() != Some(text) => {
                        return fail(index, &format!("uses {{{{secret:{secret}}}}} outside a fill step's value"));
                    }
                    Some(secret) if secret.is_empty() || !secret.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                        return fail(index, "names a secret by letters, numbers, and underscores");
                    }
                    Some(_) => {}
                    None if !steps.inputs.contains_key(name) => return fail(index, &format!("uses {{{{{name}}}}}, which inputs doesn't list")),
                    None => {}
                }
            }
        }
        for text in step.element.iter().chain(&step.note) {
            short(text, 300, &format!("{PATH}: step {}", index + 1))?;
        }
    }
    Ok(steps)
}

impl Steps {
    /// The steps with the run's inputs in place of their `{{name}}`s. Every input the steps use
    /// must be given.
    pub fn with_inputs(&self, inputs: &BTreeMap<String, String>) -> Result<Steps, String> {
        let used: std::collections::BTreeSet<&str> = self.steps.iter().flat_map(Step::texts).flat_map(|text| placeholders(text)).filter(|name| secret_name(name).is_none()).collect();
        let missing: Vec<&str> = used.iter().copied().filter(|name| !inputs.contains_key(*name)).collect();
        if !missing.is_empty() {
            return Err(format!("Give the run these inputs: {}.", missing.iter().map(|name| format!("{name} ({})", self.inputs.get(*name).map(String::as_str).unwrap_or_default())).collect::<Vec<_>>().join(", ")));
        }
        for (name, value) in inputs {
            if value.chars().count() > MAX_VALUE {
                return Err(format!("The input {name} is longer than {MAX_VALUE} characters."));
            }
        }
        let mut steps = self.clone();
        for step in &mut steps.steps {
            let texts = step.url.iter_mut().chain(step.value.iter_mut()).chain(step.values.iter_mut()).chain(step.files.iter_mut());
            let expected = step.expect.iter_mut().flat_map(|e| e.url.iter_mut().chain(e.title.iter_mut()).chain(e.text.iter_mut()));
            for text in texts.chain(expected) {
                *text = fill(text, inputs);
            }
            // An input fills in a web address, never another kind of page.
            if step.url.as_deref().is_some_and(|url| !(url.starts_with("https://") || url.starts_with("http://"))) {
                return Err("A step's address must be an http or https address once its inputs are filled in.".into());
            }
        }
        Ok(steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(steps: serde_json::Value) -> String {
        serde_json::json!({ "profile": "Work", "inputs": { "campaign": "The campaign's name" }, "steps": steps }).to_string()
    }

    #[test]
    fn a_steps_file_says_what_each_step_lacks() {
        let good = file(serde_json::json!([
            { "action": "goto", "url": "https://ads.example.com/" },
            { "action": "click", "element": "“New” button", "targets": ["getByRole('button', { name: 'New', exact: true })", "locator('#new')"], "expect": { "url": "/new" } },
            { "action": "fill", "targets": ["getByLabel('Name', { exact: true })"], "value": "{{campaign}}" },
            { "action": "fill", "targets": ["getByLabel('Password', { exact: true })"], "secret": true },
            { "action": "press", "key": "Enter" },
        ]));
        let steps = parse(&good).unwrap();
        assert_eq!(steps.steps.len(), 5);
        let typed = parse(&file(serde_json::json!([{ "action": "fill", "targets": ["#pw"], "value": "{{secret:SHOP_PASSWORD}}" }]))).unwrap();
        assert_eq!(secrets_named(&typed), ["SHOP_PASSWORD"]);
        assert_eq!(typed.with_inputs(&BTreeMap::new()).unwrap().steps[0].value.as_deref(), Some("{{secret:SHOP_PASSWORD}}"), "a run fills a secret in at the browser, not here");
        assert_eq!(steps.steps[1].describe(), "click “New” button");
        assert_eq!(steps.steps[3].describe(), "type a password into getByLabel('Password', { exact: true })");
        for (bad, why) in [
            (file(serde_json::json!([{ "action": "click" }])), "step 1 needs 1 to 8 targets"),
            (file(serde_json::json!([{ "action": "goto", "url": "file:///etc/passwd" }])), "needs an http or https url"),
            (file(serde_json::json!([{ "action": "fill", "targets": ["#a"] }])), "needs a value, or secret: true"),
            (file(serde_json::json!([{ "action": "fill", "targets": ["#a"], "value": "{{nope}}" }])), "uses {{nope}}"),
            (file(serde_json::json!([{ "action": "click", "targets": ["#a"], "expect": {} }])), "expects nothing"),
            (file(serde_json::json!([{ "action": "click", "targets": ["#a"], "timeout": 600 }])), "waits 1 to 60"),
            (file(serde_json::json!([{ "action": "press", "key": "Enter", "targets": ["#a"] }])), "takes no targets"),
            (file(serde_json::json!([{ "action": "click", "targets": ["#a"], "page": {} }])), "unknown field"),
            (file(serde_json::json!([])), "1 to 100 steps"),
            (file(serde_json::json!([{ "action": "goto", "url": "https://x.example/?p={{secret:PW}}" }])), "outside a fill step's value"),
            (file(serde_json::json!([{ "action": "fill", "targets": ["#pw"], "value": "{{secret:pass word}}" }])), "letters, numbers, and underscores"),
        ] {
            let error = parse(&bad).unwrap_err();
            assert!(error.contains(why), "{error} should say {why}");
        }
    }

    #[test]
    fn a_run_fills_in_its_inputs_and_needs_every_one_it_uses() {
        let steps = parse(&file(serde_json::json!([
            { "action": "fill", "targets": ["#name"], "value": "Spring {{campaign}}" },
            { "action": "click", "targets": ["#create"], "expect": { "text": "{{campaign}} created" } },
        ]))).unwrap();
        assert!(steps.with_inputs(&BTreeMap::new()).unwrap_err().contains("campaign (The campaign's name)"));
        let filled = steps.with_inputs(&BTreeMap::from([("campaign".to_string(), "sale".to_string())])).unwrap();
        assert_eq!(filled.steps[0].value.as_deref(), Some("Spring sale"));
        assert_eq!(filled.steps[1].expect.as_ref().unwrap().text.as_deref(), Some("sale created"));
        let opens = parse(&file(serde_json::json!([{ "action": "goto", "url": "{{campaign}}" }]))).unwrap();
        assert!(opens.with_inputs(&BTreeMap::from([("campaign".to_string(), "javascript:alert(1)".to_string())])).unwrap_err().contains("http or https"));
        assert!(opens.with_inputs(&BTreeMap::from([("campaign".to_string(), "https://ads.example.com/".to_string())])).is_ok());
    }
}
