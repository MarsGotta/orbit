import { localizeMessage, type ErrorOrigin, type Message } from "./interceptor";

/** The message of a refused order sent from `place`, the newest if several. */
export function messageFor(place: ErrorOrigin, messages: readonly Message[]): Message | undefined {
  const message = messages.filter((message) => message.place === place).at(-1);
  return message ? localizeMessage(message) : undefined;
}

/** The messages of the banner strip: everything not shown where it was sent from. */
export function bannerMessages(messages: readonly Message[]): Message[] {
  return messages.filter((message) => message.place === undefined);
}
