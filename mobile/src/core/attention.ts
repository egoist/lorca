// What waits on the user across chats, as the Rust core projects it (`attention.changed`); the
// encrypted records stay in the core.
export interface AttentionRevision { counter: number; device_id: string }
export interface AttentionSource { chat_id: string; task_id?: string; message_id?: string; review_id?: string }
export interface AttentionItem {
  id: string; category: "review" | "blocker" | "commitment" | "change"; title: string; summary: string;
  next_action: string; coordinator_bot_id: string; sources: AttentionSource[]; urgent: boolean; revision: AttentionRevision;
}
export interface AttentionBrief {
  coordinator_bot_id: string; chat_id: string; decisions: string[]; changes: string[]; next_action: string; updated_at: number;
}
export interface AttentionPreferences { summaries: boolean; urgent_direct: boolean; default_coordinator_bot_id: string | null }
export interface AttentionView { items: AttentionItem[]; briefs: AttentionBrief[]; preferences: AttentionPreferences }
export function emptyAttention(): AttentionView {
  return { items: [], briefs: [], preferences: { summaries: true, urgent_direct: true, default_coordinator_bot_id: null } };
}
