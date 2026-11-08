import { numberFormat } from "../locale";
// The floating panel's own words: asking the agent without speaking, its
// answer and what it asks to do. Named after the conversation's agent.
export function floatingPanelCopy(assistantName: string) {
  return {
    floatingPanel: {
      askAboutCall: "Ask about the call",
      askAboutMeeting: "Ask about the meeting",
      listenLast: "Listen to the last",
      minutes: (minutes: number) => `${numberFormat().format(minutes)} min`,
      voiceHint: `Say “Hey ${assistantName}” and your question`,
      alsoInConversation: "Also saved in your conversation",
      another: "Another question",
      close: "Close",
      // The approval choices, short to fit the panel's 340 px.
      choices: {
        once: "Allow once",
        session: "In this conversation",
        always: "Always",
        deny: "No",
      },
    },
  } as const;
}
