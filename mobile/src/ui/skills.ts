// A bot's or a group's skills on the phone, after the desktop apps' skill sheet and Save as Skill:
// what the screens word, the name the core takes, and the form the skill screen and its file
// screen share while a skill is open.

import { create } from "zustand";
import { canBeQuoted, type Chat, type Message, type PlaybookContent, type PlaybookFile, type PlaybookRecord, type PlaybookRevision, type PlaybookScope } from "../core/model";
import { t } from "../i18n";

/// The name as the core takes it: lowercase, hyphens for spaces.
export function skillSlug(name: string): string {
  return name.trim().toLowerCase().replace(/[ _]/g, "-");
}

/// Why the core would refuse the name, in words for the form.
export function skillNameProblem(name: string): string | undefined {
  const slug = skillSlug(name);
  if (!slug) return undefined;
  const ok = slug.length <= 64 && /^[a-z0-9-]+$/.test(slug) && !slug.startsWith("-") && !slug.endsWith("-") && !slug.includes("--");
  return ok ? undefined : t("Use lowercase letters, numbers, and hyphens.");
}

/// A bundled file's name the core takes: letters, numbers, dots, hyphens, and underscores, in
/// folders, none of them hidden.
export function safeFileName(name: string): boolean {
  return !!name && name.split("/").every((part) => !!part && !part.startsWith(".") && /^[A-Za-z0-9._-]+$/.test(part));
}

/// Who can use the skill, and for a draft, that it waits for a save.
export function skillScopeLine(scope: PlaybookScope, isDraft: boolean, names: { bot?: string; group?: string }): string {
  if (scope.kind === "project") {
    const group = names.group ?? t("Group");
    return isDraft ? t("A draft. Once you save it, the bots in {group} can use it there.", { group }) : t("The bots in {group} can use it there.", { group });
  }
  const bot = names.bot ?? t("The bot");
  return isDraft ? t("A draft. Once you save it, {bot} can use it in every chat.", { bot }) : t("{bot} can use it in every chat.", { bot });
}

/// What happened at a step of a skill's history, in a word or three.
export function historyWord(step: PlaybookRevision, steps: PlaybookRevision[]): string {
  if (step.status === "deleted") return t("Deleted");
  if (step.status === "draft") return step.provenance.kind === "corrections" ? t("Drafted from corrections") : t("Drafted from a chat");
  if (step.provenance.kind === "edit" && step.revision > 1) return steps.some((before) => before.revision === step.revision - 1 && before.status === "draft") ? t("Saved") : t("Edited");
  return t("Created");
}

/// The messages Save as Skill offers in a chat: the user's and the bots' completed text, the
/// last twenty, oldest first.
export function captureSources(chat: Chat): Message[] {
  return chat.messages.filter((m) => canBeQuoted(m) && m.body.kind === "text" && m.body.text.trim().length > 0).slice(-20);
}

/// What the picked messages make: a workflow from a bot's reply (with the bot that wrote it), or a
/// standing instruction from two or more of the user's corrections (for the group's owner, or
/// the DM's bot). Undefined while the picks are not enough for either.
export function captureKind(chat: Chat, picked: Message[]): { kind: "workflow" | "corrections"; botId: string } | undefined {
  const reply = picked.find((m) => m.author.kind === "bot");
  if (reply && reply.author.kind === "bot") return { kind: "workflow", botId: reply.author.bot_id };
  const owner = chat.owner_bot_id ?? chat.bot_ids[0];
  if (picked.length >= 2 && picked.every((m) => m.author.kind === "you") && owner) return { kind: "corrections", botId: owner };
  return undefined;
}

/// A bundled file being edited: its folder, its name in it, and its text. Its key stays through
/// renames.
export interface SkillFile {
  key: number;
  folder: "references" | "scripts";
  name: string;
  text: string;
}

interface SkillForm {
  name: string;
  description: string;
  instructions: string;
  examples: string;
  files: SkillFile[];
}

/// The open skill's form, which its file screen edits too.
export const useSkillForm = create<SkillForm>(() => ({ name: "", description: "", instructions: "", examples: "", files: [] }));

let nextKey = 0;

export function fillSkillForm(content: PlaybookContent | null | undefined) {
  const files = (folder: "references" | "scripts", list: PlaybookFile[]) =>
    list.map((file) => ({ key: ++nextKey, folder, name: file.path.slice(folder.length + 1), text: file.text }));
  useSkillForm.setState({
    name: content?.name ?? "",
    description: content?.description ?? "",
    instructions: content?.instructions ?? "",
    examples: content?.examples ?? "",
    files: [...files("references", content?.references ?? []), ...files("scripts", content?.scripts ?? [])],
  });
}

/// A new file in the folder, under a name no other file there has.
export function addSkillFile(folder: "references" | "scripts"): number {
  const { files } = useSkillForm.getState();
  const [stem, ext] = folder === "scripts" ? ["script", "sh"] : ["notes", "md"];
  let name = `${stem}.${ext}`;
  for (let n = 2; files.some((f) => f.folder === folder && f.name === name); n++) name = `${stem}-${n}.${ext}`;
  const key = ++nextKey;
  useSkillForm.setState({ files: [...files, { key, folder, name, text: "" }] });
  return key;
}

export function skillFormContent(form: SkillForm): PlaybookContent {
  const files = (folder: "references" | "scripts") => form.files.filter((f) => f.folder === folder).map((f) => ({ path: `${folder}/${f.name}`, text: f.text }));
  return { name: skillSlug(form.name), description: form.description.trim(), instructions: form.instructions, examples: form.examples, references: files("references"), scripts: files("scripts") };
}

/// Whether the form says something other than the record.
export function skillFormChanged(form: SkillForm, record: PlaybookRecord | undefined): boolean {
  return JSON.stringify(skillFormContent(form)) !== JSON.stringify(record?.content ?? skillFormContent({ name: "", description: "", instructions: "", examples: "", files: [] }));
}
