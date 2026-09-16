//! A terminal chat over the harness: the built-in coding tools in the current directory,
//! skills from `./skills`, prompt templates from `./prompts`, and a few slash commands.
//!
//! ```text
//! DEEPSEEK_API_KEY=… cargo run -p tinybot-agent --example chat -- deepseek/deepseek-flash
//! ANTHROPIC_API_KEY=… cargo run -p tinybot-agent --example chat -- anthropic/claude-opus-5
//! ```
//!
//! Commands: `/model provider/id`, `/think level`, `/compact`, `/stats`, `/tools`, `/skill name`,
//! `/name args…` for a template, `/quit`.

use std::io::{BufRead, Write};
use std::sync::Arc;

use tinybot_agent::harness::{
    build_system_prompt, load_prompt_templates, load_skills, parse_command_args, AgentHarness, EnvProviderFactory, HarnessEvent, HarnessOptions,
    ModelIdentity, ProviderFactory, Resources, SystemPromptParts,
};
use tinybot_agent::provider::AssistantEvent;
use tinybot_agent::tools::coding_tools;
use tinybot_agent::{AgentMessage, ThinkingLevel};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = ModelIdentity::parse(&std::env::args().nth(1).unwrap_or_else(|| "deepseek/deepseek-flash".into()));
    let factory: Arc<dyn ProviderFactory> = Arc::new(EnvProviderFactory::new());
    let provider = factory.provider(&model, None)?;
    let cwd = std::env::current_dir()?;

    let skills = load_skills(&[cwd.join("skills")]);
    let templates = load_prompt_templates(&[cwd.join("prompts")]);
    for problem in skills.diagnostics.iter().map(|d| format!("{}: {}", d.path.display(), d.message)).chain(templates.diagnostics.iter().map(|d| format!("{}: {}", d.path.display(), d.message))) {
        eprintln!("warning: {problem}");
    }
    let date = chrono_free_today();
    let system_prompt = build_system_prompt(&SystemPromptParts {
        identity: None,
        cwd: Some(&cwd),
        instructions: &[],
        coding_tools: true,
        skills: &skills.skills,
        date: Some(&date),
    });

    let mut options = HarnessOptions::new(provider);
    options.factory = Some(factory);
    options.system_prompt = system_prompt;
    options.tools = coding_tools(cwd.clone());
    options.resources = Resources { skills: skills.skills, prompt_templates: templates.templates };
    let mut harness = AgentHarness::new(options);

    // Print the reply as it streams, and what the tools do.
    let mut events = harness.subscribe();
    let printer = tokio::spawn(async move {
        let mut out = std::io::stdout();
        while let Some(event) = events.recv().await {
            match event {
                HarnessEvent::MessageUpdate { event: AssistantEvent::TextDelta { delta, .. }, .. } => {
                    print!("{delta}");
                    out.flush().ok();
                }
                HarnessEvent::MessageEnd { message: AgentMessage::Assistant(m), .. } => {
                    if let Some(error) = &m.error_message {
                        println!("\n[{error}]");
                    } else {
                        println!();
                    }
                }
                HarnessEvent::ToolStart { tool_name, args, .. } => println!("\n[{tool_name}] {args}"),
                HarnessEvent::RetryScheduled { attempt, max_attempts, delay_ms, error, .. } => println!("[retry {attempt}/{max_attempts} in {delay_ms} ms: {error}]"),
                HarnessEvent::CompactionEnd { reason, tokens_before, .. } => println!("[compacted ({reason:?}), {} tokens before]", tokens_before.unwrap_or(0)),
                HarnessEvent::Usage { usage, totals } => {
                    eprintln!("[{} in, {} out, ${:.4}; ${:.4} so far]", usage.input + usage.cache_read, usage.output, usage.cost.total, totals.cost.total)
                }
                _ => {}
            }
        }
    });

    let stdin = std::io::stdin();
    loop {
        print!("\n> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let result = if let Some(command) = line.strip_prefix('/') {
            let (name, rest) = command.split_once(' ').unwrap_or((command, ""));
            match name {
                "quit" | "exit" => break,
                "model" => harness.set_model(&ModelIdentity::parse(rest.trim())).await.map(|_| None),
                "think" => match rest.trim().parse::<ThinkingLevel>() {
                    Ok(level) => harness.set_thinking_level(Some(level)).await.map(|_| None),
                    Err(error) => {
                        println!("{error}");
                        continue;
                    }
                },
                "compact" => harness.compact(None).await.map(|outcome| {
                    println!("{:?}", outcome.outcome);
                    None
                }),
                "stats" => {
                    let stats = harness.stats();
                    println!("{} · thinking {:?} · context {} of {} tokens · {} messages · ${:.4}", stats.model, stats.thinking_level, stats.context_tokens, stats.context_window, stats.messages, stats.totals.cost.total);
                    continue;
                }
                "tools" => {
                    println!("{}", harness.active_tools().join(", "));
                    continue;
                }
                "skill" => harness.skill(rest.trim(), None).await.map(Some),
                other => harness.prompt_from_template(other, &parse_command_args(rest)).await.map(Some),
            }
        } else {
            harness.prompt(line).await.map(Some)
        };
        match result {
            Ok(Some(run)) => eprintln!("[{:?}]", run.outcome),
            Ok(None) => {}
            Err(error) => println!("{error}"),
        }
    }
    drop(harness);
    let _ = printer.await;
    Ok(())
}

/// Today's date without a date crate: days since the epoch, civil from days.
fn chrono_free_today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
