import type { NoteLibrarySnapshot } from "../../bridge/generated/NoteLibrarySnapshot";
import type { NoteTemplateDetailSnapshot } from "../../bridge/generated/NoteTemplateDetailSnapshot";

const available = { state: "available", detail: null } as const;

const templates: NoteTemplateDetailSnapshot[] = [
  {
    id: "general",
    name: "Reunión general",
    context: "Una reunión de trabajo cualquiera.",
    sections: [
      { name: "Contexto", instructions: "Por qué se reunieron y qué había que decidir." },
      { name: "Lo que se habló", instructions: "Los puntos principales, en viñetas." },
      { name: "Próximos pasos", instructions: "Qué hará cada cual, en una lista numerada." },
    ],
    builtin: true,
    forked_from: null,
    pending: false,
  },
  {
    id: "one-on-one",
    name: "Uno a uno",
    context: "Una conversación entre una responsable y una persona de su equipo.",
    sections: [
      { name: "Cómo está", instructions: "Ánimo, carga de trabajo y lo que le preocupa." },
      { name: "Acuerdos", instructions: "Lo que queda decidido entre las dos personas." },
    ],
    builtin: true,
    forked_from: null,
    pending: false,
  },
  {
    id: "interview",
    name: "Entrevista de usuario",
    context: "Una entrevista para entender cómo trabaja alguien.",
    sections: [
      { name: "Quién es", instructions: "Su puesto y su día a día." },
      { name: "Problemas", instructions: "Lo que le cuesta, con sus palabras." },
      { name: "Citas", instructions: "Frases literales que valga la pena guardar." },
    ],
    builtin: true,
    forked_from: null,
    pending: false,
  },
  {
    id: "design-review",
    name: "Revisión de diseño",
    context: "Una revisión de pantallas con diseño, producto e ingeniería.",
    sections: [
      { name: "Contexto", instructions: "Qué pantallas se revisaron y para qué entrega." },
      { name: "Lo que se habló", instructions: "Las opiniones sobre cada propuesta, con datos." },
      { name: "Próximos pasos", instructions: "Los cambios pedidos y quién los hace." },
    ],
    builtin: false,
    forked_from: { id: "general", name: "Reunión general" },
    pending: false,
  },
];

export const library: NoteLibrarySnapshot = {
  dictionaries: available,
  dictionary_items: [
    {
      dictionary: {
        id: "dict-nubaria",
        name: "Nubaria",
        terms: [
          { id: "t1", correct: "Nubaria", wrong: ["nuvaria", "nube aria"] },
          { id: "t2", correct: "Hoteles Ribera", wrong: ["hoteles rivera"] },
          { id: "t3", correct: "check-in", wrong: ["chekin", "chequin"] },
          { id: "t4", correct: "Sara Llorente", wrong: ["sara yorente"] },
        ],
        series: [
          { calendar_item_id: "item-design-review", title: "Revisión del diseño de reservas" },
        ],
      },
      pending: false,
    },
    {
      dictionary: {
        id: "dict-tech",
        name: "Ingeniería",
        terms: [
          { id: "t5", correct: "Postgres", wrong: ["postgre", "post gres"] },
          { id: "t6", correct: "Kubernetes", wrong: ["cubernetes"] },
        ],
        series: [],
      },
      pending: false,
    },
  ],
  dictionary_pending: [],
  dictionary_failures: [],
  read: "available",
  read_detail: null,
  templates: available,
  items: templates,
  links: available,
  assignments: [
    {
      calendar_item_id: "item-design-review",
      template_id: "design-review",
      template_name: "Revisión de diseño",
      pending: false,
    },
    {
      calendar_item_id: "item-daily",
      template_id: "general",
      template_name: "Reunión general",
      pending: false,
    },
  ],
  refusals: [],
  restores: [],
  bounds: {
    max_sections: 12,
    max_name_chars: 80,
    max_context_chars: 2000,
    max_instructions_chars: 1000,
    max_custom_templates: 32,
  },
};
