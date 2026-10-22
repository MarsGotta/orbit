// The floating panel's own words: asking the agent without speaking, its
// answer and what it asks to do. Named after the conversation's agent.
export function floatingPanelCopy(assistantName: string) {
  return {
    floatingPanel: {
      askAboutCall: "Pregunta sobre la llamada",
      askAboutMeeting: "Pregunta sobre la reunión",
      listenLast: "Escucha los últimos",
      minutes: (minutes: number) => `${minutes} min`,
      voiceHint: `Di «Oye ${assistantName}» y tu pregunta`,
      alsoInConversation: "Queda también en tu conversación",
      another: "Otra pregunta",
      close: "Cerrar",
      // The approval choices, short to fit the panel's 340 px.
      choices: {
        once: "Permitir una vez",
        session: "En esta conversación",
        always: "Siempre",
        deny: "No",
      },
    },
  } as const;
}
