export const errors = {
  errors: {
    unknownCode: (code: string) => `Orbit does not recognise this code: ${code}.`,
    disconnectedNotes: "Connect Orbit to the hub to read your notes.",
    notesNoResponse: "The hub did not respond to the Notes request. Try again.",
    notesReadFailed: "Couldn’t read Notes.",
    title: "Couldn’t complete this",
  },
} as const;
