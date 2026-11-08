import type { AppCommand } from "../../../bridge/generated";
import type { NoteLibrarySnapshot } from "../../../bridge/generated/NoteLibrarySnapshot";
import type { NoteTemplateDetailSnapshot } from "../../../bridge/generated/NoteTemplateDetailSnapshot";
import type { TemplateSection } from "../../../bridge/generated/TemplateSection";

/** What the user edits of a template. */
export interface TemplateDraft {
  name: string;
  context: string;
  sections: TemplateSection[];
}

/**
 * Who the edits go to: the template itself, or — for one of Orbit's — a copy
 * of the user's own, made on the first change, that names where it comes from.
 */
export interface EditTarget {
  id: string;
  forkedFrom: string | null;
}

export function draftOf(template: NoteTemplateDetailSnapshot): TemplateDraft {
  return {
    name: template.name,
    context: template.context,
    sections: template.sections.map(({ instructions, name }) => ({ instructions, name })),
  };
}

/** The id of a new template of the user's: chosen here, so saving twice is one. */
export function newTemplateId(): string {
  return crypto.randomUUID();
}

/** A section moved from one place to another, the rest in their order. */
export function moveSection<T>(sections: T[], from: number, to: number): T[] {
  if (from === to || from < 0 || to < 0 || from >= sections.length || to >= sections.length)
    return sections;
  const next = [...sections];
  next.splice(to, 0, ...next.splice(from, 1));
  return next;
}

export function saveCommand(target: EditTarget, draft: TemplateDraft): AppCommand {
  return {
    command: "note_library",
    request: {
      action: "save_template",
      template_id: target.id,
      draft: {
        name: draft.name,
        context: draft.context,
        sections: draft.sections,
        forked_from: target.forkedFrom,
      },
    },
  };
}

/**
 * What «Deshacer» needs of a template the user just deleted — the
 * template as the library held it, under its id, and the series that used it
 * (`null` when the assignments were not known).
 */
export interface RemovedTemplate extends TemplateDraft {
  id: string;
  forkedFrom: string | null;
  series: string[] | null;
}

export function removedTemplate(
  library: NoteLibrarySnapshot,
  id: string,
  draft: TemplateDraft,
): RemovedTemplate {
  const held = library.items.find((item) => item.id === id);
  const origin = held?.forked_from?.id ?? null;
  return {
    id,
    // What the library held; text typed and not saved was dropped with it.
    ...(held ? draftOf(held) : draft),
    // The hub makes a copy again only from an origin it still holds: one
    // gone from the list would refuse the whole template, so it comes back
    // without it.
    forkedFrom: origin !== null && library.items.some((item) => item.id === origin) ? origin : null,
    series:
      library.links.state === "available"
        ? library.assignments
            .filter((link) => link.template_id === id)
            .map((link) => link.calendar_item_id)
        : null,
  };
}

export function restoreCommand(removed: RemovedTemplate, requestId: string): AppCommand {
  return {
    command: "note_library",
    request: {
      action: "restore_template",
      template_id: removed.id,
      request_id: requestId,
      ...(removed.series === null ? {} : { series: removed.series }),
      draft: {
        name: removed.name,
        context: removed.context,
        sections: removed.sections,
        forked_from: removed.forkedFrom,
      },
    },
  };
}
