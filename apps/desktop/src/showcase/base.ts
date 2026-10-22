import type { AppBootstrap, FeatureFrame, FeatureId } from "../bridge/generated";
import { agenda } from "./data/agenda";
import { agentConfigs, skillDocuments } from "./data/agents";
import { briefing } from "./data/briefing";
import { chat, conversation, SESSION, sessions } from "./data/chat";
import { control } from "./data/control";
import { library } from "./data/library";
import { readOf, reviewNote } from "./data/note-open";
import { noteList } from "./data/notes-list";
import { recording } from "./data/recording";
import { local, permissions, status } from "./data/status";
import type { HubData, Views } from "./scenario";

/** An ordinary Wednesday morning: everything works and nothing is recording. */
export function baseViews(): Views {
  return structuredClone({
    status,
    permissions,
    agenda,
    notes: {
      list: noteList,
      index: { state: "available", more: true, detail: null },
      open: null,
      library,
    },
    note: null,
    recording,
    assistant: chat,
    control,
    briefing,
    local,
  });
}

export function baseHub(): HubData {
  return structuredClone({
    sessions,
    messages: { [SESSION]: conversation },
    agents: agentConfigs,
    skills: skillDocuments,
  });
}

/** The note a scenario opens, with its read finished. */
export function withOpenNote(views: Views, note = reviewNote): Views {
  const notes = views.notes ?? { list: [], open: null };
  return { ...views, note, notes: { ...notes, open: note, open_read: readOf(note) } };
}

const FEATURES: FeatureId[] = [
  "status",
  "permissions",
  "agenda",
  "notes",
  "note",
  "recording",
  "assistant",
  "control",
  "briefing",
  "local",
];

/** The frames of `views`, each at `counter`. */
export function framesOf(views: Partial<Views>, counter: number): FeatureFrame[] {
  const revision = { epoch: 1, counter };
  return FEATURES.filter((feature) => feature in views).map(
    (feature) => ({ feature, revision, view: views[feature] }) as FeatureFrame,
  );
}

export function bootstrapOf(views: Views): AppBootstrap {
  return { revision: { epoch: 1, counter: 1 }, frames: framesOf(views, 1) };
}
