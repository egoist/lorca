//! A general agent on top of the loop, after pi's coding-agent harness: a transcript in memory
//! (the host persists what it wants from the events), a model and thinking level that can
//! change between turns, tools, skills and prompt templates, compaction, retry, queues, hooks,
//! and events.

pub mod events;
pub mod factory;
pub mod frontmatter;
#[allow(clippy::module_inception)]
pub mod harness;
pub mod hooks;
pub mod skills;
pub mod system_prompt;
pub mod templates;

pub use events::{EventBus, HarnessEvent, HarnessListener, Outcome, QueueKind, QueuedItem};
pub use factory::{EnvProviderFactory, ModelIdentity, ProviderFactory};
pub use harness::{convert_with_summaries, AgentHarness, CompactionOutcome, HarnessError, HarnessHandle, HarnessOptions, HarnessStats, RunResult};
pub use hooks::{
    AfterToolDecision, BeforeToolDecision, BlockedTool, CompactionDecision, CompactionReason, HarnessHooks, HookRegistry,
    RequestStep, Resources,
};
pub use skills::{format_skill_invocation, format_skills_for_system_prompt, load_skills, LoadedSkills, Skill, SkillDiagnostic};
pub use system_prompt::{build_system_prompt, SystemPromptParts};
pub use templates::{
    format_prompt_template_invocation, load_prompt_templates, parse_command_args, substitute_args, LoadedTemplates, PromptTemplate,
    TemplateDiagnostic,
};
