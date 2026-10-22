/** The Markdown editor every note, answer, section, personality and template opens in. */
export const editorCopy = {
  editor: {
    placeholder: "Escribe o pulsa / para insertar…",
    help: [
      ["/", "insertar"],
      ["[[", "enlazar una nota"],
      ["@", "mencionar"],
      ["**", "negrita"],
      ["- [ ]", "tarea"],
      ["⌘/", "ver el Markdown"],
    ] as const,
    helpLabel: "Atajos del editor",
    markdown: "Markdown",
    backToText: "Volver al texto",
    showMarkdown: "Ver el Markdown · ⌘/",
    showFormatted: "Ver con formato · ⌘/",
    rawLabel: (label: string) => `${label} en Markdown`,

    // The «/» menu: each option and the Markdown it writes.
    insert: "Insertar",
    heading: "Título",
    subheading: "Subtítulo",
    bullet: "Lista",
    ordered: "Lista numerada",
    task: "Tarea",
    quote: "Cita",
    code: "Código",
    rule: "Separador",
    linkNote: "Enlazar una nota",
    mention: "Mencionar a alguien",
    keepWriting: (name: string) => `Pedir a ${name} que siga escribiendo`,
    nothingNamed: "Nada se llama así. Sigue escribiendo para dejarlo como texto.",
    menuKeys: ["↑↓ elegir", "↵ insertar", "esc cerrar"] as const,

    // «[[» and «@».
    linkMenu: "Enlazar una nota",
    noNoteNamed: "Ninguna nota se llama así.",
    mentionMenu: "Mencionar",
    nobodyNamed: "Nadie se llama así en tus reuniones.",
    invited: "Invitado a esta reunión",
    spoke: "Habló en esta reunión",
    known: "De tu calendario",

    // The bar over a selection.
    formatBar: "Dar formato al texto elegido",
    bold: "Negrita · ⌘B",
    italic: "Cursiva · ⌘I",
    inlineCode: "Código · ⌘E",
    linkTo: "Enlazar a una nota · ⌘K",
    highlight: "Resaltar · ⌘⇧H",
    ask: (name: string) => `Preguntar a ${name}`,

    // A task's box.
    markTask: "Marcar tarea",

    // What an assistant wrote, until it is accepted.
    writtenBy: (name: string) => `Escrito por ${name}`,
    accept: "Aceptar",
    discard: "Descartar",
    keepWritingAsk: "Sigue escribiendo esta nota a partir de aquí:",

    // Saving.
    saving: "Guardando…",
    savedHere: "Guardado en este Mac · solo lo ves tú",
    savedHub: "✓ Guardado",
    onlyHere: "Los cambios solo están aquí hasta que copies o mandes",
    notSaved: (why: string) => `No se ha guardado: ${why} Lo escrito sigue aquí.`,
    notSavedUnknown: "No se ha guardado. Lo escrito sigue aquí.",
    retrySave: "Reintentar",
    nearLimit: (left: number, max: number) =>
      `Quedan ${left.toLocaleString("es-ES")} de ${max.toLocaleString("es-ES")} caracteres.`,
    atLimit: (max: number) =>
      `Has llegado al máximo de ${max.toLocaleString("es-ES")} caracteres: lo que sigue no se escribe.`,
  },
} as const;
