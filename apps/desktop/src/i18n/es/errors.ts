export const errors = {
  errors: {
    unknownCode: (code: string) => `Orbit no reconoce este código: ${code}.`,
    disconnectedNotes: "Conecta Orbit con el hub para leer tus notas.",
    notesNoResponse: "El hub no respondió a la petición de Notas. Vuelve a intentarlo.",
    notesReadFailed: "No se pudo leer Notas.",
    title: "No se pudo completar",
  },
} as const;
