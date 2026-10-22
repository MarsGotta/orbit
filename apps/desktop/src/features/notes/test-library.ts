// The invented library of the notes tests: templates and dictionaries.
import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { NoteTemplateDetailSnapshot } from "../../bridge/generated/NoteTemplateDetailSnapshot";

const template = (id: string, name: string): NoteTemplateDetailSnapshot => ({
  id,
  name,
  context: "",
  sections: [],
  builtin: true,
  forked_from: null,
  pending: false,
});

function dictionaries(): NoteLibrarySnapshot["dictionary_items"] {
  return [
    {
      dictionary: {
        id: "dic-biblio",
        name: "Biblioteca",
        terms: [],
        series: [
          { calendar_item_id: "serie-comite", title: "Comité" },
          { calendar_item_id: "serie-obras", title: "Obras" },
        ],
      },
      pending: false,
    },
    { dictionary: { id: "dic-otro", name: "Otro", terms: [], series: [] }, pending: false },
  ];
}

export function library(): NoteLibrarySnapshot {
  return {
    read: "available",
    read_detail: null,
    dictionaries: { state: "available", detail: null },
    dictionary_items: dictionaries(),
    dictionary_pending: [],
    dictionary_failures: [],
    templates: { state: "available", detail: null },
    items: [
      template("general", "General"),
      template("review", "Revisión"),
      template("interview", "Entrevista"),
    ],
    links: { state: "available", detail: null },
    assignments: [],
    refusals: [],
    restores: [],
    bounds: {
      max_sections: 12,
      max_name_chars: 80,
      max_context_chars: 4000,
      max_instructions_chars: 2000,
      max_custom_templates: 32,
    },
  };
}
