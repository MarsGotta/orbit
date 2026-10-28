// These literals carry machine semantics, or belong to the separate error-code
// migration. Matching the exact file and value prevents a growing copy baseline.
export const exceptions = {
  "features/assistant/useAttachmentDrafts.ts": ["FileReader failed"], // Internal FileReader diagnostic.
  "features/notes/action-groups.ts": ["Yo"], // Recognises an owner alias in meeting content.
  "features/notes/NoteSettingsFields.tsx": ["reuniones"], // Settings route key.
  "features/notes/note-settings.ts": ["en", "de"], // Meeting language codes.
  "features/vault/VaultMarkdown.tsx": ["No se pudo leer esta consulta de tareas."], // Error-code migration.
  "features/vault/api.ts": [
    "Conecta Orbit con el hub para leer tus notas.",
    "El hub no respondió a la petición de Notas. Vuelve a intentarlo.",
    "No se pudo leer Notas.",
  ], // Error-code migration.
  "state/ui.ts": ["reuniones", "notas"], // Settings route keys.
  "state/orbit-store.ts": ["subscribing to frames", "reading the first view"], // Internal timeout diagnostics.
};
