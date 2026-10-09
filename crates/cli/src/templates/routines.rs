//! Scheduling configuration goes through the canonical Routine/API, including #78's additive
//! time zone and outage policy. Unsupported recipient capability blocks setup before mutation.

use super::format::Routine;
use crate::app::App;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn export(source: crate::model::Routine) -> Routine {
    let wire = serde_json::to_value(&source).unwrap_or_default();
    Routine {
        name: source.name,
        schedule: source.schedule,
        prompt: source.prompt,
        check: source.check,
        timezone: wire["timezone"].as_str().map(str::to_string),
        missed_run_policy: wire["missed_run_policy"].as_str().map(str::to_string),
    }
}

pub async fn validate(app: &Arc<App>, routine: &Routine) -> Result<(), String> {
    let request = json!({ "schedule": routine.schedule, "timezone": routine.timezone, "missed_run_policy": routine.missed_run_policy });
    let response = Box::pin(crate::api::dispatch(app, "routines.describe", request)).await?;
    for (field, supplied) in [
        ("timezone", routine.timezone.is_some()),
        ("missed_run_policy", routine.missed_run_policy.is_some()),
    ] {
        if supplied && response.get(field).is_none() {
            return Err(format!("Routine {:?} requires unsupported scheduling field {field}. Update this CLI before importing it.", routine.name));
        }
    }
    Ok(())
}

pub async fn create_paused(
    app: &Arc<App>,
    bot_id: &str,
    routine: &Routine,
) -> Result<Value, String> {
    let mut request = serde_json::to_value(routine).map_err(|e| e.to_string())?;
    request["bot_id"] = json!(bot_id);
    request["enabled"] = json!(false);
    Box::pin(crate::api::dispatch(app, "routines.create", request)).await
}
