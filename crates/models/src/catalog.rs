//! The catalog as data. `catalog.json` is read by rules every later version keeps, so a catalog
//! written for a newer Lorca never breaks an older one, and the catalog in use is replaced whole
//! by a newer one.
//!
//! The rules: fields this version does not know are ignored; a model it cannot run as listed (a
//! required field missing, a thinking mode or wire it does not know, an id listed twice) is left
//! out, and thinking levels it does not know are dropped from a model. A catalog is refused
//! whole when its `version` is not one this build reads, its `updated` is not a UTC time, or a
//! provider it or the bundled catalog lists has no `review` model among its models.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, RwLock};

use serde::Deserialize;
use serde_json::Value;

use crate::{CostTier, ModelInfo, Rates, ThinkingLevel, ThinkingMode, Wire};

/// The catalog this build carries.
pub const BUNDLED: &str = include_str!("../catalog.json");

/// The format this build reads. A change older builds cannot read gets a new version, served
/// beside this one.
const VERSION: u64 = 1;

/// A catalog as [`parse`] read it.
#[derive(Debug, Clone, PartialEq)]
pub struct Catalog {
    /// When it last changed, as `YYYY-MM-DDTHH:MM:SSZ`; of two catalogs the later one wins.
    pub updated: String,
    /// In order: a provider's first model is its default.
    pub models: Vec<ModelInfo>,
    /// The model Auto-review runs on, by provider.
    pub review: BTreeMap<String, String>,
}

/// The bundled catalog: the first in use, and the providers every later one must keep.
static BUNDLED_CATALOG: LazyLock<&'static Catalog> = LazyLock::new(|| Box::leak(Box::new(read(BUNDLED, &BTreeSet::new()).expect("the bundled catalog reads"))));

static CURRENT: LazyLock<RwLock<&'static Catalog>> = LazyLock::new(|| RwLock::new(*BUNDLED_CATALOG));

/// Reads a catalog by the rules. The strings of the models it keeps are leaked, so lookups can
/// hand out `&'static` entries: a few kilobytes for each catalog a process reads, which happens
/// at startup and when a newer one is fetched.
pub fn parse(json: &str) -> Result<Catalog, String> {
    let bundled: BTreeSet<&str> = BUNDLED_CATALOG.models.iter().map(|m| m.provider).collect();
    read(json, &bundled)
}

/// Makes `catalog` the one every lookup reads when its `updated` is later than the one in use.
/// True when it was. The old catalog is never freed, as entries handed out before may still be
/// in use.
pub fn install(catalog: Catalog) -> bool {
    let mut current = CURRENT.write().unwrap();
    if catalog.updated <= current.updated {
        return false;
    }
    *current = Box::leak(Box::new(catalog));
    true
}

/// When the catalog in use last changed.
pub fn updated() -> &'static str {
    &current().updated
}

pub(crate) fn current() -> &'static Catalog {
    *CURRENT.read().unwrap()
}

#[derive(Deserialize)]
struct RawCatalog {
    version: u64,
    updated: String,
    models: Vec<Value>,
    #[serde(default)]
    review: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct RawModel {
    provider: String,
    id: String,
    name: String,
    context_window: u64,
    max_output: u64,
    #[serde(default)]
    reasoning: bool,
    #[serde(default)]
    images: bool,
    rates: Rates,
    #[serde(default)]
    tiers: Vec<CostTier>,
    /// Absent for a decision model, which does not think out loud.
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    levels: Vec<String>,
    #[serde(default)]
    wire: Option<String>,
}

fn read(json: &str, required: &BTreeSet<&str>) -> Result<Catalog, String> {
    let raw: RawCatalog = serde_json::from_str(json).map_err(|e| format!("The model catalog does not read: {e}"))?;
    if raw.version != VERSION {
        return Err(format!("The model catalog is version {}; this Lorca reads version {VERSION}", raw.version));
    }
    if !is_utc_time(&raw.updated) {
        return Err(format!("The model catalog's updated time {:?} is not YYYY-MM-DDTHH:MM:SSZ", raw.updated));
    }
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();
    for value in raw.models {
        let Ok(entry) = serde_json::from_value::<RawModel>(value) else { continue };
        if entry.provider.is_empty() || entry.id.is_empty() || !seen.insert((entry.provider.clone(), entry.id.clone())) {
            continue;
        }
        if let Some(model) = model(entry) {
            models.push(model);
        }
    }
    let providers: BTreeSet<&str> = models.iter().map(|m| m.provider).chain(required.iter().copied()).collect();
    for provider in providers {
        if !raw.review.contains_key(provider) {
            return Err(format!("The model catalog has no review model for {provider}"));
        }
    }
    for (provider, id) in &raw.review {
        if !models.iter().any(|m| m.provider == provider.as_str() && m.id == id.as_str()) {
            return Err(format!("The model catalog's review model {provider}/{id} is not among its models"));
        }
    }
    Ok(Catalog { updated: raw.updated, models, review: raw.review })
}

/// One model, or nothing when this version cannot run it as listed.
fn model(entry: RawModel) -> Option<ModelInfo> {
    let wire: Option<Wire> = match entry.wire {
        Some(wire) => Some(wire.parse().ok()?),
        None => None,
    };
    let thinking: ThinkingMode = match entry.thinking {
        Some(thinking) => thinking.parse().ok()?,
        // A decision model is asked nothing about thinking; the mode is never read.
        None if wire == Some(Wire::SystemOne) => ThinkingMode::Effort,
        None => return None,
    };
    let mut levels: Vec<ThinkingLevel> = entry.levels.iter().filter_map(|level| level.parse().ok()).collect();
    levels.sort();
    levels.dedup();
    Some(ModelInfo {
        id: entry.id.leak(),
        name: entry.name.leak(),
        provider: entry.provider.leak(),
        context_window: entry.context_window,
        max_output: entry.max_output,
        reasoning: entry.reasoning,
        images: entry.images,
        rates: entry.rates,
        tiers: entry.tiers.leak(),
        thinking,
        levels: levels.leak(),
        wire,
    })
}

/// `YYYY-MM-DDTHH:MM:SSZ`, which sorts as the times it names.
fn is_utc_time(text: &str) -> bool {
    let pattern = b"dddd-dd-ddTdd:dd:ddZ";
    text.len() == pattern.len() && text.bytes().zip(pattern).all(|(c, p)| if *p == b'd' { c.is_ascii_digit() } else { c == *p })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The bundled catalog with `models` and `review` replaced.
    fn catalog(models: Value, review: Value) -> String {
        let mut catalog: Value = serde_json::from_str(BUNDLED).unwrap();
        catalog["models"] = models;
        catalog["review"] = review;
        catalog.to_string()
    }

    /// Whole-dollar rates are written as integers, as the site's minified copy has them.
    fn entry(provider: &str, id: &str) -> Value {
        json!({
            "provider": provider, "id": id, "name": id, "context_window": 200_000, "max_output": 32_000,
            "rates": { "input": 1, "output": 2, "cache_read": 0.1, "cache_write": 0 },
            "thinking": "effort", "levels": ["low", "high"]
        })
    }

    /// One model for each provider the bundled catalog has, each its own review model.
    fn every_provider() -> (Vec<Value>, Value) {
        let providers: BTreeSet<&str> = BUNDLED_CATALOG.models.iter().map(|m| m.provider).collect();
        let models = providers.iter().map(|p| entry(p, "base")).collect();
        let review = providers.iter().map(|p| (p.to_string(), json!("base"))).collect::<serde_json::Map<_, _>>();
        (models, Value::Object(review))
    }

    #[test]
    fn the_bundled_catalog_reads_whole() {
        let raw: Value = serde_json::from_str(BUNDLED).unwrap();
        assert_eq!(BUNDLED_CATALOG.models.len(), raw["models"].as_array().unwrap().len(), "the rules left a bundled model out");
        assert!(is_utc_time(&BUNDLED_CATALOG.updated));
        // Every OpenCode model names its wire, so no catalog model is routed by its id.
        for model in BUNDLED_CATALOG.models.iter().filter(|m| m.provider.starts_with("opencode")) {
            assert!(model.wire.is_some(), "{}/{} has no wire", model.provider, model.id);
        }
    }

    #[test]
    fn a_model_this_version_cannot_run_is_left_out() {
        let (mut models, review) = every_provider();
        let mut unknown_mode = entry("deepseek", "new-mode");
        unknown_mode["thinking"] = json!("interleaved");
        let mut unknown_wire = entry("opencode", "new-wire");
        unknown_wire["wire"] = json!("realtime");
        let mut no_rates = entry("deepseek", "no-rates");
        no_rates.as_object_mut().unwrap().remove("rates");
        let mut extras = entry("opencode", "extras");
        extras["levels"] = json!(["max", "ultra", "low", "low"]);
        extras["wire"] = json!("messages");
        extras["badge"] = json!("New");
        // Only a decision model goes without a thinking mode.
        let mut no_thinking = entry("opencode", "no-thinking");
        no_thinking.as_object_mut().unwrap().remove("thinking");
        let mut decides = no_thinking.clone();
        decides["id"] = json!("decides");
        decides["wire"] = json!("system-one");
        models.extend([unknown_mode, unknown_wire, no_rates, extras, no_thinking, decides, entry("deepseek", "base")]);
        let parsed = parse(&catalog(Value::Array(models), review)).unwrap();

        let ids: Vec<(&str, &str)> = parsed.models.iter().map(|m| (m.provider, m.id)).collect();
        assert!(!ids.contains(&("deepseek", "new-mode")) && !ids.contains(&("opencode", "new-wire")) && !ids.contains(&("deepseek", "no-rates")));
        assert!(!ids.contains(&("opencode", "no-thinking")));
        assert!(parsed.models.iter().find(|m| m.id == "decides").is_some_and(ModelInfo::decides));
        assert_eq!(ids.iter().filter(|id| **id == ("deepseek", "base")).count(), 1, "an id listed twice keeps its first entry");
        let extras = parsed.models.iter().find(|m| m.id == "extras").unwrap();
        assert_eq!(extras.levels, &[ThinkingLevel::Low, ThinkingLevel::Max]);
        assert_eq!((extras.thinking, extras.wire), (ThinkingMode::Effort, Some(Wire::Messages)));
    }

    #[test]
    fn a_catalog_that_breaks_the_rules_is_refused() {
        let (models, review) = every_provider();
        let good = catalog(Value::Array(models.clone()), review.clone());
        assert!(parse(&good).is_ok());

        let mut newer: Value = serde_json::from_str(&good).unwrap();
        newer["version"] = json!(2);
        assert!(parse(&newer.to_string()).unwrap_err().contains("version 2"));

        let mut local: Value = serde_json::from_str(&good).unwrap();
        local["updated"] = json!("2026-10-02T09:00:00+08:00");
        assert!(parse(&local.to_string()).is_err());

        // A provider this build has may not go missing, and each review model must be listed.
        let without_grok: Vec<Value> = models.iter().filter(|m| m["provider"] != "grok").cloned().collect();
        let mut review_without_grok = review.clone();
        review_without_grok.as_object_mut().unwrap().remove("grok");
        assert!(parse(&catalog(Value::Array(without_grok), review_without_grok)).unwrap_err().contains("grok"));
        let mut unlisted = review.clone();
        unlisted["anthropic"] = json!("claude-gone");
        assert!(parse(&catalog(Value::Array(models.clone()), unlisted)).unwrap_err().contains("anthropic/claude-gone"));

        // A provider this build does not know yet is fine, given its own review model.
        let mut more = models.clone();
        more.push(entry("mistral", "large"));
        assert!(parse(&catalog(Value::Array(more.clone()), review.clone())).is_err());
        let mut more_review = review.clone();
        more_review["mistral"] = json!("large");
        assert!(parse(&catalog(Value::Array(more), more_review)).is_ok());
    }

    #[test]
    fn only_a_later_catalog_is_installed() {
        // The bundled models under a later time, so the lookups other tests make answer the same.
        let mut later: Value = serde_json::from_str(BUNDLED).unwrap();
        later["updated"] = json!("2999-01-01T00:00:00Z");
        assert!(install(parse(&later.to_string()).unwrap()));
        assert_eq!(updated(), "2999-01-01T00:00:00Z");
        assert!(!install(parse(BUNDLED).unwrap()), "an older catalog never replaces a newer one");
        assert!(!install(parse(&later.to_string()).unwrap()), "nor does the same one again");
        assert_eq!(updated(), "2999-01-01T00:00:00Z");
    }
}
