//! Intake: a channel decides which messages become work, instead of giving each one a full turn.
//!
//! A message goes through a gate in code (who wrote it, and whether it addressed the bot), then
//! a read-only assessment by a small model against the owner's rules, which ends in a recorded
//! [`Decision`]. An accepted message becomes a durable [Task](crate::tasks) that carries its
//! [`Mandate`]: why the work may be done and how far it may go. The assessment holds no tools and
//! no state, so nothing a message says can act; code checks what it answers. See
//! `docs/architecture/intake.md`.

#[cfg(feature = "runner")]
mod assess;
#[cfg(feature = "runner")]
mod decisions;
#[cfg(feature = "runner")]
mod run;
#[cfg(all(test, feature = "runner", feature = "server"))]
mod tests;

use serde::{Deserialize, Serialize};

#[cfg(feature = "runner")]
pub use decisions::{forget_channel, list as decisions_of, Decision, DecisionKind};
#[cfg(feature = "runner")]
pub use run::*;

/// A Task's run answered that nothing needs doing: it ends quietly, and a later message can
/// resume it. Any other cancelled Task was stopped by someone, and stays stopped.
pub const NOTHING_TO_DO: &str = "Nothing to do for this request.";
/// A mandated run that ended by failing, not by anyone's choice, which starts again by itself.
pub const RUN_FAILED: &str = "Its run failed before it finished.";
pub const STOPPED_BY_USER: &str = "Stopped by the user.";

// A Device that runs no bots has no Task runs to ask about.
#[cfg(not(feature = "runner"))]
pub fn reached_thread(_app: &crate::app::App, _task: &crate::tasks::Task, _since: f64) -> bool {
    false
}
#[cfg(not(feature = "runner"))]
pub fn background_live(_app: &crate::app::App, _task: &crate::tasks::Task) -> bool {
    false
}

/// The most rules and people a channel's intake holds.
pub const MAX_RULES: usize = 16;
pub const MAX_PEOPLE: usize = 64;
pub const MAX_RULE_NAME: usize = 60;
pub const MAX_RULE_INSTRUCTION: usize = 1_000;

/// What a channel takes on by itself, in the owner's words. A channel without one gives each
/// message it takes a turn.
/// Fields it doesn't know are ignored: a Runner's machine blob carries this to every Device, and one
/// that a newer Runner adds a field to still reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IntakeSpec {
    #[serde(default)]
    pub mode: Mode,
    /// Who may ask the bot for work directly, by the service's own user id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<Person>,
    /// The kinds of work the bot takes on when nobody asked it directly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
}

/// What an assessment that matches a rule does. A direct request from a listed person is taken
/// in either mode.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Each message the channel takes starts a turn, as without intake. The people and rules stay
    /// for when it is turned on again.
    Off,
    /// Records what the bot would have taken on, and does nothing.
    #[default]
    Observe,
    /// Takes it on.
    Auto,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Person {
    /// Telegram's numeric user id, Slack's `U…`. A name can be changed by its owner, so only this
    /// counts.
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// The kind of work and how far it goes. Process and communication it asks for hold for the
    /// Tasks it accepts.
    pub instruction: String,
}

fn enabled() -> bool {
    true
}

impl IntakeSpec {
    pub fn enabled_rules(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter().filter(|rule| rule.enabled)
    }

    pub fn listed(&self, sender_id: &str) -> bool {
        !sender_id.is_empty() && self.people.iter().any(|person| person.id == sender_id)
    }

    /// Identifies this version of the owner's words, which each Mandate records and which
    /// discards an assessment in flight when it changes.
    pub fn version(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        crate::keys::b64(&<sha2::Sha256 as sha2::Digest>::digest(&bytes))[..16].to_string()
    }

    /// Checks what the owner wrote, and gives each rule without an id one from its name.
    pub fn checked(mut self) -> Result<IntakeSpec, String> {
        if self.rules.len() > MAX_RULES || self.people.len() > MAX_PEOPLE {
            return Err(format!("Use at most {MAX_RULES} rules and {MAX_PEOPLE} people."));
        }
        for person in &mut self.people {
            person.id = person.id.trim().to_string();
            person.name = person.name.trim().to_string();
            if person.id.is_empty() || person.id.len() > 64 {
                return Err("Name each person by the service's user id.".into());
            }
        }
        let mut ids: Vec<String> = Vec::new();
        for rule in &mut self.rules {
            rule.name = rule.name.trim().to_string();
            rule.instruction = rule.instruction.trim().to_string();
            if rule.name.is_empty() || rule.name.chars().count() > MAX_RULE_NAME {
                return Err(format!("Give each rule a name of at most {MAX_RULE_NAME} characters."));
            }
            if rule.instruction.is_empty() || rule.instruction.chars().count() > MAX_RULE_INSTRUCTION {
                return Err(format!("Give each rule an instruction of at most {MAX_RULE_INSTRUCTION} characters."));
            }
            if rule.id.trim().is_empty() {
                let digest = <sha2::Sha256 as sha2::Digest>::digest(rule.name.as_bytes());
                rule.id = format!("rule-{}", &hex(&digest)[..8]);
            }
            if ids.contains(&rule.id) {
                return Err(format!("Two rules share the id {}.", rule.id));
            }
            ids.push(rule.id.clone());
        }
        Ok(self)
    }

    /// The people and rules in words, for the bot's tool.
    pub fn describe(&self) -> String {
        let mode = match self.mode {
            Mode::Off => "off",
            Mode::Observe => "observing",
            Mode::Auto => "taking on work",
        };
        format!("{mode}, {} people, {} rules", self.people.len(), self.enabled_rules().count())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// How strong the ground for a message is, in code, before any model reads it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    /// A listed person addressed the bot: they asked. No rule is needed, in either mode.
    Explicit,
    /// Anything else: only a matching rule can make it work, and only in `auto`.
    Inferred,
}

/// What the gate makes of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// A model reads it, with this strength.
    Assess(Strength),
    /// Addressed by someone unlisted with no rule it could match: the channel's own task answers,
    /// and no model is asked first.
    Turn,
    /// Not addressed, and no rule it could match: recorded and left alone, and no model is asked.
    Leave,
}

/// The gate: code only, so nothing in the message's words reaches it.
pub fn gate(spec: &IntakeSpec, sender_id: &str, addressed: bool) -> Gate {
    let rules = spec.enabled_rules().next().is_some();
    match (spec.listed(sender_id), addressed, rules) {
        (true, true, _) => Gate::Assess(Strength::Explicit),
        (_, _, true) => Gate::Assess(Strength::Inferred),
        (_, true, false) => Gate::Turn,
        (_, false, false) => Gate::Leave,
    }
}

/// What an accept grants a Task: why the work may be done, and how far it may go. It is set once,
/// by the host, and travels with the Task to every Device; only a new accept can widen it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Mandate {
    pub channel_id: String,
    pub channel_name: String,
    /// The channel's task as it stood, in the owner's words.
    pub channel_task: String,
    /// `telegram` or `slack`.
    pub service: String,
    /// The Task's own chat, which holds its runs apart from every other Task's. People's messages
    /// reach it by code; they never see it.
    pub work_chat_id: String,
    /// The conversation it came from, and the contact's message that started it.
    pub chat_id: String,
    pub message_id: String,
    /// That message's id on the service, which a reply there names.
    pub external_id: String,
    /// Who wrote it, as the service identifies them and as they call themselves.
    pub sender_id: String,
    pub sender: String,
    pub strength: Strength,
    /// The rules it matched, in the owner's words as they stood; none for a direct request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// A listed person's message, word for word: what Auto-review reads as their request. Empty
    /// for inferred work, which only the rules authorize.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request: String,
    /// The work as the assessment worded it. It comes from outside words, so it only narrows:
    /// Auto-review never reads it.
    pub scope: String,
    /// The conversation around the message when it was accepted, as data.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub context: String,
    /// The version of the channel's intake that accepted it.
    pub policy_version: String,
    /// Why the assessment accepted it, and the words it found that for.
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    pub at: i64,
}

impl Mandate {
    /// Whether the Mandate still holds under the channel's intake as it stands now, before a
    /// later message resumes its Task. A rule's words may change; it must still be on.
    pub fn holds(&self, spec: &IntakeSpec) -> bool {
        match self.strength {
            Strength::Explicit => spec.listed(&self.sender_id),
            Strength::Inferred => {
                spec.mode == Mode::Auto && !self.rules.is_empty() && self.rules.iter().all(|rule| spec.enabled_rules().any(|known| known.id == rule.id))
            }
        }
    }
}
