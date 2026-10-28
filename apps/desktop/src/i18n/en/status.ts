const base = {
  appName: "Orbit",
  loading: "Connecting to Orbit…",
  connectFailed: "Couldn’t connect to Orbit. Close and reopen it.",
  retry: "Try again",
  design: {
    askOrbit: "Command palette",
    commandInputLabel: "Command",
    commandPlaceholder: "Ask, dictate or call…",
    escape: "esc",
    undo: "Undo",
    expandCall: "Open",
    collapseCall: "Minimize",
    hangUp: "Hang up",
    callSubtitle: "Speak aloud",
  },
} as const;

/** The window's frame, with the conversation's agent where it is asked or called. */
export function statusCopy(assistantName: string) {
  return {
    ...base,
    design: {
      ...base.design,
      inCall: `On a call with ${assistantName}`,
      callAssistant: `Call ${assistantName}`,
    },
  } as const;
}
