import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import { useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { SignInAttempt } from "../../bridge/generated/SignInAttempt";

import type { AgentPort } from "../settings/agents/use-agent-config";
import { callAgent } from "./agent-calls";

/**
 * How often the Mac reads a sign-in again while it waits (hub-v1: «the Mac
 * reads it again until it is no longer `waiting`»). Two seconds says a
 * failure about as soon as the person looks back at Orbit; over the hub's 15
 * minutes it is at most 450 small reads. A success also arrives on its own
 * as the agent's `account`.
 */
const READ_AGAIN_MS = 2_000;

export type SignInView =
  /**
   * Nothing under way; `ended` says how the last attempt ended, when it did
   * not sign in, and `failed` whether it failed rather than was cancelled.
   */
  | { state: "idle"; ended?: string; failed?: boolean }
  | { state: "starting" }
  | {
      state: "waiting";
      /** The address the browser opens; shown only when the browser did not open. */
      url?: string;
      /** Codex's one-time code, typed on the vendor's page. */
      code?: string;
      /** Claude Code's page shows a code the person pastes back here. */
      expectsCode: boolean;
      browserFailed: boolean;
      sendingCode: boolean;
      codeError?: string;
    }
  | { state: "done" };

type Send = (command: AppCommand) => unknown;
type Waiting = Extract<SignInView, { state: "waiting" }>;

/** What one answer of the hub about the attempt shows, given what showed before. */
export function viewOf(
  attempt: SignInAttempt,
  before: SignInView,
  browserFailed: boolean,
  words: { vendor: string; name: string },
): SignInView {
  switch (attempt.state) {
    case "waiting":
      return {
        state: "waiting",
        url: attempt.url,
        code: attempt.code,
        expectsCode: attempt.expects_code,
        browserFailed: before.state === "waiting" ? before.browserFailed : browserFailed,
        sendingCode: false,
        codeError: before.state === "waiting" ? before.codeError : undefined,
      };
    case "done":
      return { state: "done" };
    case "cancelled":
      return {
        state: "idle",
        ended: getCopy().extras.assistantsCopy.signInEnded("cancelled", words.vendor, words.name),
        failed: false,
      };
    case "failed":
    case "unknown":
      return {
        state: "idle",
        ended: getCopy().extras.assistantsCopy.signInEnded(attempt.cause, words.vendor, words.name),
        failed: true,
      };
  }
}

/** Opens the vendor's page; `false` when the Mac could not. */
async function openInBrowser(send: Send, url: string): Promise<boolean> {
  try {
    return (await Promise.resolve(send({ command: "open_url", url }))) !== false;
  } catch {
    return false;
  }
}

/**
 * The attempt's state on this screen, and the timer that reads it again.
 * The hub's own word that the account is signed in ends the wait.
 */
function useAttempt(signedIn: boolean) {
  const [view, setView] = useState<SignInView>({ state: "idle" });
  const agentId = useRef<string | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      window.clearTimeout(timer.current);
    };
  }, []);
  const show = (next: SignInView | ((before: SignInView) => SignInView)) => {
    if (alive.current) setView(next);
  };
  const waiting = view.state === "waiting" || view.state === "starting";
  useEffect(() => {
    if (!signedIn || !waiting) return;
    window.clearTimeout(timer.current);
    setView({ state: "done" });
  }, [signedIn, waiting]);
  return { view, show, agentId, timer };
}

/**
 * One assistant's sign-in from Orbit: the hub starts the vendor's own tool
 * and answers the address (and a code); the Mac opens the browser and waits.
 * The token never reaches the Mac. Cancelling leaves the agent as it was.
 */
interface SignInOptions {
  port?: AgentPort;
  send: Send;
  vendor: string;
  name: string;
  /** The hub already says the account is signed in. */
  signedIn: boolean;
}

export function useSignIn({ port, send, vendor, name, signedIn }: SignInOptions) {
  const copy = useCopy().extras.assistantsCopy;
  const { view, show, agentId, timer } = useAttempt(signedIn);
  const words = { vendor, name };
  const ended = (text: string, failed = true) => {
    window.clearTimeout(timer.current);
    show({ state: "idle", ended: text, failed });
  };
  const follow = (attempt: SignInAttempt, browserFailed: boolean) => {
    window.clearTimeout(timer.current);
    show((before) => viewOf(attempt, before, browserFailed, words));
    if (attempt.state === "waiting")
      timer.current = window.setTimeout(() => void readAgain(), READ_AGAIN_MS);
  };
  const ask = async (call: "sign_in" | "sign_in_status" | "sign_in_cancel") => {
    const id = agentId.current;
    return id === null ? null : callAgent(port, { call, agent_id: id });
  };
  async function readAgain() {
    const result = await ask("sign_in_status");
    if (result === null) return;
    if (!result.ok) return ended(result.text);
    if (result.outcome.outcome === "sign_in") follow(result.outcome.attempt, false);
  }
  const start = async (id: string) => {
    agentId.current = id;
    show({ state: "starting" });
    const result = await ask("sign_in");
    if (result === null) return;
    if (!result.ok) return ended(result.text);
    if (result.outcome.outcome !== "sign_in") return;
    const { attempt } = result.outcome;
    const opened =
      attempt.state !== "waiting" ||
      attempt.url === undefined ||
      (await openInBrowser(send, attempt.url));
    follow(attempt, !opened);
  };
  const cancel = async () => {
    window.clearTimeout(timer.current);
    const result = await ask("sign_in_cancel");
    if (result === null) return show({ state: "idle" });
    if (!result.ok) return ended(result.text);
    ended(copy.signInEnded("cancelled", vendor, name), false);
  };
  const submitCode = (code: string) =>
    sendCode(code, { port, agentId: agentId.current, show, follow });
  return { view, start, cancel, submitCode };
}

/** Claude's code back from its page: while it goes, the button waits; a refusal says why. */
async function sendCode(
  code: string,
  {
    port,
    agentId,
    show,
    follow,
  }: {
    port?: AgentPort;
    agentId: string | null;
    show: (next: (before: SignInView) => SignInView) => void;
    follow: (attempt: SignInAttempt, browserFailed: boolean) => void;
  },
): Promise<void> {
  if (agentId === null) return;
  const waiting = (change: Partial<Waiting>) =>
    show((before) => (before.state === "waiting" ? { ...before, ...change } : before));
  waiting({ sendingCode: true, codeError: undefined });
  const result = await callAgent(port, {
    call: "sign_in_code",
    agent_id: agentId,
    code: code.trim(),
  });
  if (!result.ok) return waiting({ sendingCode: false, codeError: result.text });
  if (result.outcome.outcome === "sign_in") follow(result.outcome.attempt, false);
  else waiting({ sendingCode: false });
}

export type SignIn = ReturnType<typeof useSignIn>;
