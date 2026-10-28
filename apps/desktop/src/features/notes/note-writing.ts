import { getCopy } from "../../i18n";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";

import type { AppView } from "../../state/app-view";
import { agentFor } from "../assistant/agents";
import { insertIntoComposer } from "../assistant/composer-insert";
import { quoteFragment } from "../assistant/conversation-markdown";
import type { EditorAssistant } from "../editor/MarkdownEditor";
import { folded, type LinkTarget, type Person } from "../editor/menus";
import { formatClock, formatDayMonth, groupFor, isUnnamedSpeaker } from "./notes-format";
import type { Send } from "./send";

/** What the note's editors offer from what the app already holds. */
export interface NoteWriting {
  notes: LinkTarget[];
  people: Person[];
  assistant?: EditorAssistant;
}

/**
 * How much of the note «Pedir a … que siga escribiendo» carries: its end,
 * so that the question stays well under the hub's message ceiling and the
 * assistant reads what it goes on from.
 */
export const KEEP_WRITING_CHARS = 4_000;

/** «[[»: the other meetings' notes, by title, with their day or today's hour. */
function linkTargets(snapshot: AppView, note: OpenNoteSnapshot): LinkTarget[] {
  return (snapshot.notes?.list ?? [])
    .filter((item) => item.recording_id !== note.recording_id && item.title.trim() !== "")
    .map((item) => ({
      title: item.title,
      detail:
        groupFor(item) === "today"
          ? getCopy().notesInline.todayAt(formatClock(item.started_at_ms))
          : formatDayMonth(item.started_at_ms),
    }));
}

/** «@»: the meeting's guests, who spoke in it, then the people of the calendar. */
function peopleOf(snapshot: AppView, note: OpenNoteSnapshot): Person[] {
  const seen = new Set<string>();
  const people: Person[] = [];
  const add = (name: string | null | undefined, detail: string) => {
    const clean = name?.trim() ?? "";
    if (clean === "" || clean.includes("@")) return;
    const key = folded(clean);
    if (seen.has(key)) return;
    seen.add(key);
    people.push({ name: clean, detail });
  };
  for (const guest of note.event?.attendees ?? []) {
    if (!guest.is_self) add(guest.name, getCopy().editor.invited);
  }
  for (const speaker of note.speakers) {
    if (!isUnnamedSpeaker(speaker) && speaker.name !== note.owner_name)
      add(speaker.name, getCopy().editor.spoke);
  }
  for (const event of snapshot.calendar.events) {
    for (const guest of event.attendees ?? []) {
      if (guest.is_self !== true) add(guest.name, getCopy().editor.known);
    }
  }
  return people;
}

/** A new conversation with the principal, `text` waiting in its box to be sent. */
function openWithPrincipal(send: Send, agentId: string | undefined, text: string): void {
  void send(
    agentId === undefined
      ? { command: "new_session" }
      : { command: "new_session", agent_id: agentId },
  );
  insertIntoComposer(text);
  window.location.hash = "#/assistant";
}

export function noteWriting(
  snapshot: AppView | null | undefined,
  note: OpenNoteSnapshot,
  send: Send,
): NoteWriting {
  if (!snapshot) return { notes: [], people: [] };
  const principal = agentFor(snapshot);
  const name = principal?.name ?? snapshot.settings.assistant_name ?? getCopy().shell.assistantName;
  return {
    notes: linkTargets(snapshot, note),
    people: peopleOf(snapshot, note),
    assistant: {
      name,
      ask: (selection) => openWithPrincipal(send, principal?.id, quoteFragment(selection)),
      keepWriting: (text) =>
        openWithPrincipal(
          send,
          principal?.id,
          `${getCopy().editor.keepWritingAsk}\n\n${quoteFragment(text.slice(-KEEP_WRITING_CHARS))}`,
        ),
    },
  };
}
