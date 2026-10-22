import type { AppCommand } from "../bridge/generated";
import { withOpenNote } from "./base";
import { reviewNote } from "./data/note-open";
import type { Views } from "./scenario";

/**
 * The few orders the showcase's daemon answers with a new state, so a click
 * through the views opens something; every other order is only accepted.
 */
export function reactTo(views: Views, command: AppCommand): Partial<Views> | null {
  switch (command.command) {
    case "notes_open": {
      const listed = views.notes?.list.find((note) => note.recording_id === command.recording_id);
      if (!listed) return null;
      const note = { ...reviewNote, recording_id: listed.recording_id, title: listed.title };
      const opened = withOpenNote(views, note);
      return { note: opened.note, notes: opened.notes };
    }
    case "session_open":
      return {
        assistant: {
          ...views.assistant,
          session_id: command.id,
          agent_id: command.agent_id,
          turn: [],
        },
      };
    case "new_session":
      return {
        assistant: {
          ...views.assistant,
          session_id: null,
          agent_id: command.agent_id,
          turn: [],
          work: undefined,
          pending: null,
        },
      };
    default:
      return null;
  }
}
