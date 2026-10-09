use crate::app::App;
use async_trait::async_trait;
use lorca_agent::{Tool, ToolError, ToolResult, ToolUpdateFn};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) struct TasksTool {
    pub app: Arc<App>,
    pub bot_id: String,
    pub chat_id: String,
    pub job_id: String,
}

#[async_trait]
impl Tool for TasksTool {
    fn name(&self) -> &str {
        "tasks"
    }
    fn description(&self) -> &str {
        "Durable tasks persist ownership, goal, acceptance criteria, dependencies, next action, state, and result/evidence across turns and compaction. list/get inspect current records. create makes queued work; run claims a single turn on the owning bot's Runner. update changes only supplied fields, using expected_revision from the latest record; stale edits fail. request_id identifies a mutation: reuse it with identical arguments only when retrying delivery. blocked/cancelled need reason; completed needs result and evidence. Evidence kinds message/output/file/review reference records in linked chats; url uses HTTPS. For a file or link you published for the task (publish_output with its task_id), pass the task_evidence object publish_output returned; a task run records the outputs it published as evidence when it ends. A child handoff references task_id without claiming the parent run."
    }
    fn parameters(&self) -> Value {
        json!({
            "type":"object","required":["action"],"properties":{
            "action":{"type":"string","enum":["list","get","create","update","run"]},
            "all":{"type":"boolean","description":"list: inspect all tasks across chats; an explicit owner filter also spans chats"},
            "id":{"type":"string"},"request_id":{"type":"string"},"expected_revision":{"type":"integer"},
            "refresh":{"type":"boolean","description":"get: refresh from the authority Runner (default true); false reads the local replica"},
                "owner_bot_id":{"type":"string"},"runner_id":{"type":"string"},"chat_id":{"type":"string"},
                "goal":{"type":"string"},"next_action":{"type":"string"},"acceptance_criteria":{"type":"array","items":{"type":"string"}},
                "dependencies":{"type":"array","items":{"type":"string"}},"chat_ids":{"type":"array","items":{"type":"string"}},
                "state":{"type":"string","enum":["queued","working","blocked","awaiting_review","completed","cancelled"]},
                "reason":{"type":["string","null"]},"result":{"type":["string","null"]},
                "links":{"type":"array","items":{"type":"object","required":["label","url"],"properties":{"label":{"type":"string"},"url":{"type":"string"}}}},
                "evidence":{"type":"array","items":{"type":"object","required":["kind","label"],"properties":{
                    "kind":{"type":"string","enum":["message","output","file","url","review"]},"label":{"type":"string"},
                    "chat_id":{"type":"string"},"message_id":{"type":"string"},"attachment_id":{"type":"string"},"url":{"type":"string"},
                    "output_id":{"type":"string"},"version":{"type":"integer"},"review_id":{"type":"string"}
                }}}
            }
        })
    }
    async fn execute(
        &self,
        _id: &str,
        mut args: Value,
        _cancel: CancellationToken,
        _update: ToolUpdateFn,
    ) -> Result<ToolResult, ToolError> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| ToolError("missing action".into()))?
            .to_string();
        let object = args
            .as_object_mut()
            .ok_or_else(|| ToolError("Task arguments must be an object".into()))?;
        object.remove("action");
        if action == "create" {
            object.entry("owner_bot_id").or_insert(json!(self.bot_id));
            object.entry("chat_ids").or_insert(json!([self.chat_id]));
        }
        let all = object
            .remove("all")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if action == "list" && !all && !object.contains_key("owner_bot_id") {
            object.entry("chat_id").or_insert(json!(self.chat_id));
        }
        if action == "get" {
            object.entry("refresh").or_insert(json!(true));
        }
        // A task run that records its own outcome says so, and the authority leaves its turn
        // to end on this result instead of cancelling it.
        if action == "update" && self.job_id.starts_with("task-run-") {
            object.insert("run_id".into(), json!(self.job_id));
        }
        // Explicit caller keys remain stable for retries; loop call ids can change after an
        // ambiguous delivery, so models must supply request_id for every mutation.
        let result = super::dispatch(&self.app, &format!("tasks.{action}"), args)
            .await
            .map_err(ToolError)?;
        let stops_this_run = action == "update"
            && result["active_run"]["id"] == self.job_id
            && result["state"] != "working";
        let result = ToolResult::text(serde_json::to_string_pretty(&result).unwrap_or_default())
            .with_details(json!({"summary":format!("Tasks · {action}"),"task":result}));
        Ok(if stops_this_run {
            result.terminating()
        } else {
            result
        })
    }
}
