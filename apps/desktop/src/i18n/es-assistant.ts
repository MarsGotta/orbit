export const esAssistant = {
  call: "Llamar",
  listening: "Escuchando…",
  microphoneLevel: "Nivel del micrófono",
  voiceNote: "Nota de voz",
  playVoiceNote: "Reproducir nota de voz",
  callEnded: "Llamada terminada",
  sessions: "Conversaciones",
  newSession: "Nueva conversación",
  activeSessions: "Activas",
  recentSessions: "Recientes",
  emptySessionTitle: "Conversación nueva",
  /** The open conversation; the list names each one by its date. */
  openSessionTitle: "Conversación",
  /** The agent a new conversation is with, when the hub has several. */
  agentPicker: "Hablar con",
  conversationWith: (name: string) => `Conversación con ${name}`,
  composerPlaceholder: "Escribe o usa el micrófono para dictar…",
  clarifyPlaceholder: "Escribe tu respuesta…",
} as const;
