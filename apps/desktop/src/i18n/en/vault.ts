/** Annex reading screens. Writing and Mac-sync promises wait for their implementation. */
export const vaultCopy = {
  title: "Notes",
  aside: "Your vault",
  home: "Home",
  tasks: "Tasks",
  map: "Map",
  folders: "Folders",
  schema: "Schema",
  index: "Index",
  search: "Search your notes…",
  projects: "Projects",
  due: "Due this week",
  allTasks: "View all",
  recent: "Recent",
  createTitle: "Your notes, in Markdown",
  createLead:
    "Connect Fast Note Sync or a folder on the hub server. Orbit keeps a read-only copy; your files stay as they are.",
  emptyAside: "No vault yet. Connect Fast Note Sync or a hub folder on the right.",
  format: "Open format: each note has a type in its properties (OKF 0.2), with an index and a log.",
  settingsIntro: "Your vault: where it lives, how meetings reach it and who maintains it.",
  readOnly: "Read-only. Actions that change your notes are coming later.",
  md: "Markdown",
  text: "Back to text",
  yaml: "Properties (YAML) · OKF 0.2",
  about: "About this note",
  props: "Properties",
  sources: "Sources",
  backlinks: "Backlinks",
  history: "History",
  noSources: "No sources yet.",
  noHistory: "No changes for this note in log.md.",
  types: {
    person: "Person",
    project: "Project",
    meeting: "Meeting",
    topic: "Theme",
    source: "Source",
    journal: "Journal",
    note: "Standalone note",
  },
  folderNames: {
    People: "People",
    Projects: "Projects",
    Meetings: "Meetings",
    Topics: "Topics",
    Sources: "Sources",
    Journal: "Journal",
  },
  tasksLead:
    "Each task lives in a note as - [ ]. This view brings together tasks from the whole vault.",
  tasksFooter:
    "New tasks come from distilled meetings, or you can write them in any note. Orbit uses a format Obsidian understands too.",
  mapLead:
    "Each dot is a note and each line links two notes. Click a note to explore its connections. Notes without links stay separate.",
  linkHelp:
    "Orbit displays both the same way. OKF uses Markdown links with paths; Obsidian uses [[ ]] by default.",
  formatHelp:
    "OKF 0.2: each note records its type, sources, author and reviewer. Includes index.md and log.md.",
  schemaHelp:
    "Note types (person, project, decision, topic…), their templates and the assistant’s rules.",
  untouched:
    "Sources are never rewritten. Your writing isn’t rewritten without asking: if new information conflicts, the assistant proposes a change.",
  phaseTwo: "Available in the writing phase. This setting doesn’t perform any action yet.",
} as const;
