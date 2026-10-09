// Workflow setups through the core: `workflows.*` keeps each one in the account's roster and runs
// its sample on the setup's Runner, and the marketplace lists the packs. The screens hold what
// they show; a change answers the setup again.

import * as core from "../../modules/lorca-core";
import type { WorkflowPack, WorkflowProgress } from "../ui/workflows";

/// The marketplace's workflows, in its order; none from a core whose index has none.
export async function loadWorkflowPacks(): Promise<WorkflowPack[]> {
  const { packs } = await core.request<{ packs?: WorkflowPack[] }>("marketplace", {});
  return packs ?? [];
}

export function workflow(method: "start" | "get" | "configure" | "connection" | "sample" | "review" | "enable" | "cancel", params: Record<string, unknown>): Promise<WorkflowProgress> {
  return core.request<WorkflowProgress>(`workflows.${method}`, params);
}
