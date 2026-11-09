import { getCopy } from "../../i18n";
import type { EditorState } from "@codemirror/state";

/** A note «[[» can link to: its title, and when it was. */
export interface LinkTarget {
  title: string;
  detail: string;
}

/** A person «@» can mention: their name, and where Orbit knows them from. */
export interface Person {
  name: string;
  detail: string;
}

/** The menu a trigger opens: what follows it so far, and the text it replaces. */
export interface Trigger {
  kind: "slash" | "link" | "mention";
  query: string;
  from: number;
  to: number;
}

/** The longest query a menu still follows: past it, it was not a trigger. */
const MAX_QUERY = 60;

/**
 * The menu the cursor is in, if any: «/» alone at the start of an empty line,
 * «[[» not closed yet, or «@» at the start of a word.
 */
export function triggerAt(state: EditorState): Trigger | null {
  const range = state.selection.main;
  if (!range.empty) return null;
  const line = state.doc.lineAt(range.head);
  const before = line.text.slice(0, range.head - line.from);
  const after = line.text.slice(range.head - line.from);
  const slash = /^\/([^\s/]*)$/.exec(before);
  if (slash && after.trim() === "") {
    return { kind: "slash", query: slash[1] ?? "", from: line.from, to: range.head };
  }
  const open = before.lastIndexOf("[[");
  if (open !== -1) {
    const query = before.slice(open + 2);
    if (!/[\][]/.test(query) && query.length <= MAX_QUERY) {
      return { kind: "link", query, from: line.from + open, to: range.head };
    }
  }
  const mention = /(?:^|[\s(])@([\p{L}'’-]*(?: [\p{L}'’-]*){0,2})$/u.exec(before);
  if (mention && (mention[1]?.length ?? 0) <= MAX_QUERY) {
    const query = mention[1] ?? "";
    return { kind: "mention", query, from: range.head - query.length - 1, to: range.head };
  }
  return null;
}

/** Lower case without accents: «Título» and «titulo» are the same search. */
export function folded(text: string): string {
  return text
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLowerCase();
}

export function matches(text: string, query: string): boolean {
  return folded(text).includes(folded(query.trim()));
}

/**
 * One option of the «/» menu: its glyph, its name, its Markdown, and what it
 * writes in place of «/…»: `before` the cursor and `after` it.
 */
export interface SlashOption {
  id: string;
  icon: string;
  label: string;
  hint: string;
  before: string;
  after?: string;
}

export function slashOptions(assistant: string | null): SlashOption[] {
  const copy = getCopy().editor;
  const options: SlashOption[] = [
    { id: "h2", icon: "T", label: copy.heading, hint: "##", before: "## " },
    { id: "h3", icon: "t", label: copy.subheading, hint: "###", before: "### " },
    { id: "bullet", icon: "•", label: copy.bullet, hint: "-", before: "- " },
    { id: "ordered", icon: "1.", label: copy.ordered, hint: "1.", before: "1. " },
    { id: "task", icon: "☐", label: copy.task, hint: "- [ ]", before: "- [ ] " },
    { id: "quote", icon: "❝", label: copy.quote, hint: ">", before: "> " },
    { id: "code", icon: "{}", label: copy.code, hint: "```", before: "```\n", after: "\n```" },
    { id: "rule", icon: "—", label: copy.rule, hint: "---", before: "---\n" },
    { id: "link", icon: "[[", label: copy.linkNote, hint: "[[", before: "[[" },
    { id: "mention", icon: "@", label: copy.mention, hint: "@", before: "@" },
  ];
  if (assistant !== null) {
    options.push({
      id: "assistant",
      icon: "✦",
      label: copy.keepWriting(assistant),
      hint: "",
      before: "",
    });
  }
  return options;
}

/** The most rows a menu lists: past them, keep typing to narrow it. */
export const MAX_MENU_ROWS = 8;

/** One row of a menu: its glyph, its name, a line under it or its Markdown on the right. */
export interface MenuRow {
  id: string;
  icon: string;
  label: string;
  detail?: string;
  hint?: string;
}

/** What a menu can list: the notes, the people and the principal's name. */
export interface MenuSources {
  notes?: readonly LinkTarget[];
  people?: readonly Person[];
  assistant?: string;
}

/** What the menu of `trigger` lists, or null when it should not open. */
export function menuRows(trigger: Trigger, sources: MenuSources): MenuRow[] | null {
  if (trigger.kind === "slash") {
    return slashOptions(sources.assistant ?? null)
      .filter((option) => matches(option.label, trigger.query))
      .map(({ id, icon, label, hint }) => ({ id, icon, label, hint }));
  }
  if (trigger.kind === "link") {
    return (sources.notes ?? [])
      .filter((note) => matches(note.title, trigger.query))
      .slice(0, MAX_MENU_ROWS)
      .map((note) => ({ id: note.title, icon: "[[", label: note.title, detail: note.detail }));
  }
  const people = (sources.people ?? [])
    .filter((person) => matches(person.name, trigger.query))
    .slice(0, MAX_MENU_ROWS)
    .map((person) => ({
      id: person.name,
      icon: person.name.slice(0, 1),
      label: person.name,
      detail: person.detail,
    }));
  // Past a space with nobody to name, «@» was not a mention.
  if (people.length === 0 && trigger.query.includes(" ")) return null;
  return people;
}

/** A menu's title, and what it says when nothing matches. */
export function menuWords(trigger: Trigger): { title: string; empty: string } {
  const copy = getCopy().editor;
  if (trigger.kind === "slash") return { title: copy.insert, empty: copy.nothingNamed };
  if (trigger.kind === "link") return { title: copy.linkMenu, empty: copy.noNoteNamed };
  return { title: copy.mentionMenu, empty: copy.nobodyNamed };
}
