import { useSyncExternalStore } from "react";

/**
 * Where the Assistant is: the open conversation (`#/assistant`, or
 * `#/assistant/c/<id>` once it has one), «¿Con quién quieres hablar?»
 * (`#/assistant/new`), «Añadir un asistente» over the view
 * (`#/assistant/add`), an assistant's own conversation
 * (`#/assistant/<agent>`) and its settings (`#/assistant/<agent>/settings`).
 * `new`, `add` and `c` are not agent ids: the hub makes `codex-…` and
 * `claude-…` ones, and a configuration that named one so would only lose
 * its own route.
 */
export type AssistantRoute =
  | { screen: "conversation"; sessionId?: string }
  | { screen: "new" }
  | { screen: "add" }
  | { screen: "agent"; agentId: string }
  | { screen: "settings"; agentId: string };

function decoded(part: string): string {
  try {
    return decodeURIComponent(part);
  } catch {
    return part;
  }
}

export function parseAssistantRoute(hash: string): AssistantRoute {
  const [view = "", first = "", second = ""] = hash.replace(/^#\/?/, "").split("/");
  if (view !== "assistant" || first === "") return { screen: "conversation" };
  if (first === "new") return { screen: "new" };
  if (first === "add") return { screen: "add" };
  if (first === "c")
    return second === ""
      ? { screen: "conversation" }
      : { screen: "conversation", sessionId: decoded(second) };
  const agentId = decoded(first);
  return second === "settings" ? { screen: "settings", agentId } : { screen: "agent", agentId };
}

export function assistantHash(route: AssistantRoute): string {
  switch (route.screen) {
    case "conversation":
      return route.sessionId === undefined
        ? "#/assistant"
        : `#/assistant/c/${encodeURIComponent(route.sessionId)}`;
    case "new":
      return "#/assistant/new";
    case "add":
      return "#/assistant/add";
    case "agent":
      return `#/assistant/${encodeURIComponent(route.agentId)}`;
    case "settings":
      return `#/assistant/${encodeURIComponent(route.agentId)}/settings`;
  }
}

/**
 * Goes to `route`. `replace` corrects the address to what is on screen (a
 * conversation that got its id) without a step of its own in the history.
 */
export function goTo(route: AssistantRoute, options: { replace?: boolean } = {}): void {
  const hash = assistantHash(route);
  if (window.location.hash === hash) return;
  if (options.replace === true) {
    window.history.replaceState(null, "", hash);
    // `replaceState` says nothing: those who read the route are told here.
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  } else window.location.hash = hash;
}

function subscribe(changed: () => void): () => void {
  window.addEventListener("hashchange", changed);
  return () => window.removeEventListener("hashchange", changed);
}

/** The route as the address says it, read again whenever it changes. */
export function useAssistantRoute(): AssistantRoute {
  const hash = useSyncExternalStore(subscribe, () => window.location.hash);
  return parseAssistantRoute(hash);
}
