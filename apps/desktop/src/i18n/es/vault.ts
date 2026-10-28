/** Annex reading screens. Writing and Mac-sync promises wait for their implementation. */
export const vaultCopy = {
  title: "Notas",
  aside: "Tu vault",
  home: "Inicio",
  tasks: "Tareas",
  map: "Mapa",
  folders: "Carpetas",
  schema: "Esquema",
  index: "Índice",
  search: "Buscar en tus notas…",
  projects: "Proyectos",
  due: "Vencen esta semana",
  allTasks: "Ver todas",
  recent: "Lo último",
  createTitle: "Tus notas, en Markdown",
  createLead:
    "Conecta Fast Note Sync o una carpeta del servidor del hub. Orbit conserva una copia de lectura; tus archivos no cambian.",
  emptyAside:
    "Aún no tienes vault. Conéctalo a la derecha con Fast Note Sync o una carpeta del hub.",
  format:
    "Formato abierto: cada nota lleva su type en las propiedades (OKF 0.2), con un índice y un registro.",
  settingsIntro: "Tu vault: dónde vive, cómo llegan a él tus reuniones y quién lo mantiene.",
  readOnly: "Solo lectura. Las acciones que cambian tus notas llegarán más adelante.",
  md: "Markdown",
  text: "Volver al texto",
  yaml: "Propiedades (YAML) · OKF 0.2",
  about: "Sobre esta nota",
  props: "Propiedades",
  sources: "Fuentes",
  backlinks: "Enlazan aquí",
  history: "Historial",
  noSources: "Aún no tiene fuentes.",
  noHistory: "No hay cambios de esta nota en log.md.",
  types: {
    person: "Persona",
    project: "Proyecto",
    meeting: "Reunión",
    topic: "Tema",
    source: "Fuente",
    journal: "Diario",
    note: "Nota suelta",
  },
  folderNames: {
    People: "Personas",
    Projects: "Proyectos",
    Meetings: "Reuniones",
    Topics: "Temas",
    Sources: "Fuentes",
    Journal: "Diario",
  },
  tasksLead: "Cada tarea vive en una nota como - [ ]. Aquí se juntan las de todo el vault.",
  tasksFooter:
    "Las tareas nuevas llegan de tus reuniones cuando se destilan, o las escribes tú en cualquier nota. Orbit las guarda en un formato que también entiende Obsidian.",
  mapLead:
    "Cada punto es una nota y cada línea, un enlace entre dos notas. Pulsa una para ver con qué se relaciona. Las notas sueltas, sin enlaces, quedan aparte.",
  linkHelp:
    "Orbit los muestra igual. Markdown con ruta es lo que pide OKF; [[ ]] es lo que usa Obsidian por defecto.",
  formatHelp:
    "OKF 0.2: cada nota lleva su type, sus fuentes y quién la escribió y la revisó. Con index.md y log.md.",
  schemaHelp:
    "Los tipos de nota (persona, proyecto, decisión, tema…), sus plantillas y las reglas que sigue el asistente.",
  untouched:
    "Las fuentes, nunca. Lo que escribes tú, no lo reescribe sin preguntar: si choca con algo nuevo, te lo propone.",
  phaseTwo: "Disponible en la fase de escritura. Este ajuste aún no ejecuta nada.",
} as const;
