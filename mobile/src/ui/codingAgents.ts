// Coding agents as the transcript, Details, and the agent's page word them: how one stands, the
// chat's agents under Coding, and its transcript's steps, with reads and searches in a row folded
// into one "Explored 3 files 2 searches" line, as Cursor shows an agent's run.

import { agentIsRunning, type AgentItem, type AgentRun, type AgentToolStep, type Message } from "../core/model";
import { t, tc } from "../i18n";

/// Its pull request was merged or closed.
function pullEnded(agent: AgentRun): boolean {
  return !!agent.pull_request && (agent.pull_state === "merged" || agent.pull_state === "closed");
}

/// How a coding agent stands, in a word or two: while it does not work, a pull request merged or
/// closed says so.
export function agentStatusWord(agent: AgentRun): string {
  if (!agentIsRunning(agent) && pullEnded(agent)) return agent.pull_state === "merged" ? tc("Merged", "coding agent") : tc("Closed", "coding agent");
  switch (agent.state) {
    case "starting":
    case "working":
      return tc("Working", "coding agent");
    case "asking":
      return tc("Needs attention", "coding agent");
    case "idle":
      if (agent.pull_state === "draft") return tc("Draft", "coding agent");
      return agent.pull_request ? tc("Ready for review", "coding agent") : tc("Finished", "coding agent");
    case "failed":
      return tc("Run failed", "coding agent");
    case "stopped":
      return tc("Stopped", "coding agent");
  }
}

/// The mark beside an agent in Details: a spinner while it works, else a symbol in a tone.
export type AgentMark = { spinner: true } | { spinner?: false; symbol: string; tone: "orange" | "red" | "tint" | "green" | "purple" | "secondary" };

export function agentMark(agent: AgentRun): AgentMark {
  if (!agentIsRunning(agent) && pullEnded(agent)) return agent.pull_state === "merged" ? { symbol: "arrow.triangle.merge", tone: "purple" } : { symbol: "xmark.circle", tone: "secondary" };
  switch (agent.state) {
    case "starting":
    case "working":
      return { spinner: true };
    case "asking":
      return { symbol: "exclamationmark.circle.fill", tone: "orange" };
    case "idle":
      return agent.pull_request ? { symbol: "arrow.triangle.pull", tone: "tint" } : { symbol: "checkmark.circle.fill", tone: "green" };
    case "failed":
      return { symbol: "exclamationmark.circle.fill", tone: "red" };
    case "stopped":
      return { symbol: "stop.circle", tone: "secondary" };
  }
}

/// An agent of the chat and the row that started it.
export type ChatAgent = { message: Message; agent: AgentRun };

/// How long an agent that ended or sits idle stays listed under Coding.
const KEEP_SECS = 3 * 24 * 3600;

/// The chat's agents for its Coding section: the ones that started, open ones (starting, working,
/// asking) first, then the latest change first; one ended or idle for more than three days is left
/// out. Agents on the same pull request are one row: the first of them in that order.
export function chatAgents(messages: Message[], now = Date.now() / 1000): ChatAgent[] {
  const listed: ChatAgent[] = [];
  for (const message of messages) {
    const agent = message.body.kind === "tool" ? message.body.agent : undefined;
    if (!agent?.id) continue;
    const changed = agent.updated_at ?? agent.started_at ?? message.created_at;
    if (!agentIsRunning(agent) && now - changed > KEEP_SECS) continue;
    listed.push({ message, agent });
  }
  const changed = (each: ChatAgent) => each.agent.updated_at ?? each.agent.started_at ?? each.message.created_at;
  listed.sort((a, b) => Number(agentIsRunning(b.agent)) - Number(agentIsRunning(a.agent)) || changed(b) - changed(a));
  const seen = new Set<string>();
  return listed.filter(({ agent }) => {
    const link = agent.pull_request?.replace(/\/+$/, "");
    if (!link) return true;
    if (seen.has(link)) return false;
    seen.add(link);
    return true;
  });
}

/// "Explored 3 files 2 searches", a part left out when it is none; "Exploring …" while one runs.
export function exploredLine(steps: AgentToolStep[], running: boolean): string {
  const count = steps.filter((step) => step.kind === "read").length;
  const searchCount = steps.length - count;
  const files = count === 0 ? "" : count === 1 ? t("1 file") : t("{count} files", { count });
  const searches = searchCount === 0 ? "" : searchCount === 1 ? t("1 search") : t("{count} searches", { count: searchCount });
  if (files && searches) return running ? t("Exploring {files} {searches}", { files, searches }) : t("Explored {files} {searches}", { files, searches });
  return running ? t("Exploring {what}", { what: files || searches }) : t("Explored {what}", { what: files || searches });
}

/// One step's line: "Ran git status", "Editing src/a.rs", "Not allowed: rm -rf build". An edit's
/// counts go beside it, in their colors.
export function toolLine(step: AgentToolStep): string {
  const title = step.title;
  if (step.status === "denied") return t("Not allowed: {title}", { title });
  const running = step.status === "running";
  switch (step.kind) {
    case "read":
      return running ? t("Reading {title}", { title }) : t("Read {title}", { title });
    case "search":
      return running ? t("Searching {title}", { title }) : t("Searched {title}", { title });
    case "edit":
      return running ? t("Editing {title}", { title }) : t("Edited {title}", { title });
    case "execute":
      return running ? t("Running {title}", { title }) : t("Ran {title}", { title });
    case "fetch":
      return running ? t("Fetching {title}", { title }) : t("Fetched {title}", { title });
    default:
      return running ? t("Using {title}", { title }) : t("Used {title}", { title });
  }
}

/// "Thought for 3s", or "Thought" when the agent did not say how long.
export function thoughtLine(seconds: number | undefined): string {
  return seconds && seconds >= 1 ? t("Thought for {seconds}s", { seconds: Math.round(seconds) }) : tc("Thought", "coding agent");
}

/// What the page draws: each step, with the reads and searches in a row as one block.
export type TranscriptBlock = { kind: "item"; item: AgentItem } | { kind: "explored"; id: string; steps: AgentToolStep[] };

export function transcriptBlocks(items: AgentItem[]): TranscriptBlock[] {
  const blocks: TranscriptBlock[] = [];
  for (const item of items) {
    const step = item.tool;
    if (item.kind === "tool" && step && (step.kind === "read" || step.kind === "search") && step.status !== "denied") {
      const last = blocks[blocks.length - 1];
      if (last?.kind === "explored") last.steps.push(step);
      else blocks.push({ kind: "explored", id: item.id, steps: [step] });
      continue;
    }
    blocks.push({ kind: "item", item });
  }
  return blocks;
}

/// " in 4 files", after the colored "+12 −3" of what an agent's work changed.
export function changedFiles(files: number): string {
  return files === 1 ? t(" in 1 file") : t(" in {count} files", { count: files });
}
