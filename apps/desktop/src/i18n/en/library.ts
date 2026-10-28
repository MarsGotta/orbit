import { numberFormat } from "../locale";
// Templates and their assignment to a series. A refusal says
// what the user can do, when they can do something; the hub's codes, numbers and
// retry times stay in the log.

/** The hub's refusals the user can act on, said as what to do. */
const libraryCodes: Record<string, string> = {
  note_templates_full: "You’ve reached the custom template limit. Delete one to create another.",
  note_templates_too_large: "Templates take too much space. Shorten one.",
  note_template_builtin: "Orbit’s templates can’t be changed.",
  note_template_not_found: "That template no longer exists.",
  note_template_not_saved: "The template wasn’t saved. Create it again.",
  note_template_links_full: "Too many series have templates. Remove one.",
  hub_unreachable:
    "No connection to the hub. The change wasn’t saved. Make it again when the hub reconnects.",
  not_sent: "The change wasn’t sent. Try again.",
};

export const library = {
  library: {
    templatesTitle: "Templates",
    fromOrbit: "by Orbit",
    chooseTemplate: "Choose a template to view or edit.",
    savesItself: "Saves automatically",
    copyOf: (name: string) => `Copy of “${name}”`,
    pending: "Saving…",
    newTemplate: "New template",
    newTemplateName: "New template",
    newSectionName: "New section",
    deleteTemplate: "Delete template",
    deleted: (name: string) => `Template “${name}” deleted. Its series now use General.`,
    restored: (name: string) => `Template “${name}” restored`,
    restoring: (name: string) => `Restoring “${name}”…`,
    // The daemon restarted before it answered the undo; the list is
    // all that is known.
    restoreUnknownAfterRestart: (name: string) =>
      `It’s unclear whether “${name}” was restored. Check the list.`,
    restoredSomeSeriesNot: (name: string) =>
      `Template “${name}” restored, but some series no longer use it.`,
    restoreRefused: (name: string) => `Couldn’t restore “${name}”.`,
    templatesFailed: "Couldn’t load templates.",
    needsHub: "Templates are read from the hub. They’ll appear when Orbit connects.",
    name: "Name",
    context: "What this meeting is about and what matters",
    contextHint: "What this meeting is about and what matters in it.",
    sections: "Note sections",
    sectionCount: (count: number, max: number) =>
      `${numberFormat().format(count)} of ${numberFormat().format(max)}`,
    sectionName: (index: number) => `Section ${numberFormat().format(index + 1)} name`,
    sectionInstructions: (index: number) =>
      `Section ${numberFormat().format(index + 1)} instructions`,
    addSection: "Add section",
    sectionsFull: (max: number) =>
      `Up to ${numberFormat().format(max)} sections. Remove one to add another.`,
    removeSection: (name: string) => `Remove “${name}”`,
    dragSection: (name: string) => `Move “${name}”: drag or use the up and down arrows`,
    builtinHint: "This is an Orbit template. Editing saves your own copy.",
    noRoom: "You’ve reached the custom template limit. Delete one to create another.",
    refused: (code: string) => libraryCodes[code] ?? "The hub rejected the change.",
    // The event panel and the note header.
    noteTemplate: "Note template",
    noteTemplateHint: "Used for notes in this meeting and the whole series.",
    useAlways: "Always use for this series",
    seriesUses: (name: string) => `This series uses “${name}”`,
  },
} as const;
