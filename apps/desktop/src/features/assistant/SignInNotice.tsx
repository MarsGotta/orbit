import { useId, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import { Button } from "../../design/Button";
import { ViewNotice } from "../../design/ViewNotice";
import { assistantsCopy as copy } from "../../i18n/es/assistants";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { healthOf } from "./assistants";
import { useSignIn, type SignInView } from "./use-sign-in";
import styles from "./Assistants.module.css";

/** Another assistant of the same service already signed in with this account. */
export function sharedAccount(agent: Agent, agents: readonly Agent[]): Agent | undefined {
  if (agent.account_email === undefined) return undefined;
  return agents.find(
    (other) =>
      other.id !== agent.id &&
      other.service === agent.service &&
      other.account_email === agent.account_email,
  );
}

/** The code Claude's page shows after signing in, pasted back into Orbit. */
function PasteCode({
  vendor,
  view,
  onSubmit,
}: {
  vendor: string;
  view: Extract<SignInView, { state: "waiting" }>;
  onSubmit: (code: string) => void;
}) {
  const [code, setCode] = useState("");
  const id = useId();
  return (
    <span className={styles.pasteCode}>
      <label className={styles.fieldLabel} htmlFor={id}>
        {copy.pasteCode}
      </label>
      <span className={styles.pasteRow}>
        <input
          aria-describedby={`${id}-help`}
          className={styles.input}
          id={id}
          onChange={(event) => setCode(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && code.trim() !== "") onSubmit(code);
          }}
          spellCheck={false}
          type="text"
          value={code}
        />
        <Button
          disabled={code.trim() === "" || view.sendingCode}
          onClick={() => onSubmit(code)}
          size={32}
        >
          {copy.sendCode}
        </Button>
      </span>
      <span className={styles.fieldHint} id={`${id}-help`}>
        {copy.pasteCodeHelp(vendor)}
      </span>
      {view.codeError !== undefined && (
        <span className={styles.fieldError} role="alert">
          {view.codeError}
        </span>
      )}
    </span>
  );
}

/** While the browser is open: what to do there, Codex's code or Claude's to paste, and «Cancelar». */
export function SignInWaiting({
  vendor,
  view,
  onCode,
  onCancel,
}: {
  vendor: string;
  view: Extract<SignInView, { state: "waiting" }> | { state: "starting" };
  onCode: (code: string) => void;
  onCancel: () => void;
}) {
  return (
    <>
      <span className={styles.block}>{copy.waitingText(vendor)}</span>
      {view.state === "waiting" && view.code !== undefined && (
        <span className={styles.deviceCode}>{copy.deviceCode(view.code)}</span>
      )}
      {view.state === "waiting" && view.browserFailed && view.url !== undefined && (
        <span className={styles.block}>
          {copy.browserNotOpened} <span className={styles.url}>{view.url}</span>
        </span>
      )}
      {view.state === "waiting" && view.expectsCode && (
        <PasteCode onSubmit={onCode} vendor={vendor} view={view} />
      )}
      <span className={styles.block}>
        <Button onClick={onCancel} size={28} variant="ghost">
          {copy.cancelSignIn}
        </Button>
      </span>
    </>
  );
}

function noticeTitle(
  place: "conversation" | "settings",
  reason: "expired" | "signed_out",
  vendor: string,
): string {
  if (place === "settings")
    return reason === "expired" ? copy.expiredBanner(vendor) : copy.signedOut(vendor);
  return reason === "expired" ? copy.signInAgainTitle(vendor) : copy.signInFirstTitle(vendor);
}

/** Signed in from here: as whom, and whether another assistant already uses that account. */
function SignedIn({ agent, agents }: { agent: Agent; agents: readonly Agent[] }) {
  const other = sharedAccount(agent, agents);
  return (
    <p className={styles.connected} role="status">
      <span className={styles.ok}>{copy.connected}</span>{" "}
      {copy.connectedAs(agent.account_email, agent.plan)}
      {other !== undefined && <span className={styles.block}>{copy.sameAccount(other.name)}</span>}
    </p>
  );
}

/** What the notice says before a sign-in, or how the last one ended. */
function noticeText(
  place: "conversation" | "settings",
  reason: "expired" | "signed_out",
  agent: Agent,
  view: SignInView,
): string {
  if (view.state === "idle" && view.ended !== undefined) return view.ended;
  if (place === "settings") return copy.expiredBannerText(agent.name);
  return reason === "expired" ? copy.signInAgainText(agent.name) : copy.signInFirstText(agent.name);
}

/**
 * An assistant that needs its session (annex 36, 36a, 36b): the amber
 * notice with «Iniciar sesión en …», then «Esperando…» while the browser is
 * open, and how it ended when it did not sign in. Once signed in it says
 * so, and whether another assistant already uses that account.
 */
export function SignInNotice({
  agent,
  agents,
  port,
  send,
  nowMs,
  place = "conversation",
}: {
  /** In its settings' «Conexión» it says what happened; in the conversation, what to do. */
  place?: "conversation" | "settings";
  agent: Agent;
  agents: readonly Agent[];
  port?: AgentPort;
  send: (command: AppCommand) => unknown;
  nowMs: number;
}) {
  const health = healthOf(agent, nowMs);
  const vendor = health.kind === "sign_in" ? health.vendor : "";
  const signedIn = agent.account === "signed_in";
  const signIn = useSignIn({ port, send, vendor, name: agent.name, signedIn });
  const { view } = signIn;
  if (view.state === "done") return <SignedIn agent={agent} agents={agents} />;
  if (health.kind !== "sign_in") return null;
  const waiting = view.state === "starting" || view.state === "waiting";
  const retry = view.state === "idle" && view.failed === true;
  return (
    <ViewNotice
      actionLabel={retry ? copy.tryAgain : copy.signIn(vendor)}
      onAction={() => void signIn.start(agent.id)}
      title={waiting ? copy.waitingTitle : noticeTitle(place, health.reason, vendor)}
      waiting={waiting}
    >
      {waiting ? (
        <SignInWaiting
          onCancel={() => void signIn.cancel()}
          onCode={(code) => void signIn.submitCode(code)}
          vendor={vendor}
          view={view}
        />
      ) : (
        noticeText(place, health.reason, agent, view)
      )}
    </ViewNotice>
  );
}
