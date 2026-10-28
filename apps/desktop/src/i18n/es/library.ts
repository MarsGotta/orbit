import { numberFormat } from "../locale";
// Templates and their assignment to a series. A refusal says
// what the user can do, when they can do something; the hub's codes, numbers and
// retry times stay in the log.

/** The hub's refusals the user can act on, said as what to do. */
const libraryCodes: Record<string, string> = {
  note_templates_full: "Ya tienes el máximo de plantillas propias: borra una para crear otra.",
  note_templates_too_large: "Las plantillas ocupan demasiado: acorta alguna.",
  note_template_builtin: "Las plantillas de Orbit no se cambian.",
  note_template_not_found: "Esa plantilla ya no existe.",
  note_template_not_saved: "La plantilla no llegó a guardarse. Vuelve a crearla.",
  note_template_links_full: "Hay demasiadas series con plantilla: quita alguna.",
  hub_unreachable: "Sin conexión con el hub: el cambio no se guardó. Hazlo otra vez cuando vuelva.",
  not_sent: "El cambio no llegó a enviarse. Hazlo otra vez.",
};

export const library = {
  library: {
    templatesTitle: "Plantillas",
    fromOrbit: "de Orbit",
    chooseTemplate: "Elige una plantilla para verla o cambiarla.",
    savesItself: "Se guarda sola",
    copyOf: (name: string) => `Copia de «${name}»`,
    pending: "Guardando…",
    newTemplate: "Nueva plantilla",
    newTemplateName: "Plantilla nueva",
    newSectionName: "Sección nueva",
    deleteTemplate: "Borrar plantilla",
    deleted: (name: string) => `Plantilla «${name}» borrada. Sus series vuelven a General.`,
    restored: (name: string) => `Plantilla «${name}» recuperada`,
    restoring: (name: string) => `Recuperando «${name}»…`,
    // The daemon restarted before it answered the undo; the list is
    // all that is known.
    restoreUnknownAfterRestart: (name: string) =>
      `No se sabe si «${name}» se recuperó. Compruébalo en la lista.`,
    restoredSomeSeriesNot: (name: string) =>
      `Plantilla «${name}» recuperada, pero alguna serie no volvió a usarla.`,
    restoreRefused: (name: string) => `No se pudo recuperar «${name}».`,
    templatesFailed: "No se pudieron cargar las plantillas.",
    needsHub: "Las plantillas se leen del hub. Aparecerán cuando Orbit se conecte.",
    name: "Nombre",
    context: "De qué va esta reunión y qué importa",
    contextHint: "Qué es esta reunión y qué importa en ella.",
    sections: "Secciones de la nota",
    sectionCount: (count: number, max: number) =>
      `${numberFormat().format(count)} de ${numberFormat().format(max)}`,
    sectionName: (index: number) => `Nombre de la sección ${numberFormat().format(index + 1)}`,
    sectionInstructions: (index: number) =>
      `Instrucciones de la sección ${numberFormat().format(index + 1)}`,
    addSection: "Añadir sección",
    sectionsFull: (max: number) =>
      `${numberFormat().format(max)} secciones como máximo. Quita una para añadir otra.`,
    removeSection: (name: string) => `Quitar «${name}»`,
    dragSection: (name: string) => `Mover «${name}»: arrástrala o usa las flechas arriba y abajo`,
    builtinHint: "Es de Orbit: si la cambias, se guarda una copia tuya.",
    noRoom: "Ya tienes el máximo de plantillas propias: borra una para crear otra.",
    refused: (code: string) => libraryCodes[code] ?? "El hub no aceptó el cambio.",
    // The event panel and the note header.
    noteTemplate: "Plantilla de la nota",
    noteTemplateHint: "Se usa en las notas de esta reunión y de toda la serie.",
    useAlways: "Usar siempre en esta serie",
    seriesUses: (name: string) => `Esta serie usa «${name}»`,
  },
} as const;
