// These literals carry machine semantics. Matching the exact file and value
// prevents a growing copy baseline.
export const exceptions = {
  "features/assistant/useAttachmentDrafts.ts": ["FileReader failed"], // Internal FileReader diagnostic.
  "features/notes/action-groups.ts": ["Yo"], // Recognises an owner alias in meeting content.
  "features/notes/NoteSettingsFields.tsx": ["reuniones"], // Settings route key.
  "features/notes/note-settings.ts": ["en", "de"], // Meeting language codes.
  "state/ui.ts": ["reuniones", "notas"], // Settings route keys.
  "state/orbit-store.ts": ["subscribing to frames", "reading the first view"], // Internal timeout diagnostics.
};
