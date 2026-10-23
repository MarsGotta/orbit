import { useState, type ReactNode } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import { Button } from "../../design/Button";
import { useFeedback } from "../../design/feedback";
import { assistantsCopy as copy, vendorOf } from "../../i18n/es/assistants";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { Group, List } from "../settings/layout";
import { callAgent } from "./agent-calls";
import { healthOf, type Health } from "./assistants";
import { agentsOf } from "./agents";
import { SignInNotice } from "./SignInNotice";
import styles from "./Assistants.module.css";

type Send = (command: AppCommand) => unknown;

/**
 * Where «Ver mi plan de …» leads: the vendor's own page of the plan, opened
 * in the browser. Only the vendors whose subscription an assistant uses.
 */
const PLAN_PAGES: Record<string, string> = {
  codex: "https://chatgpt.com/#settings/Subscription",
  "claude-code": "https://claude.ai/settings/billing",
};

/** The plan's usage reaches amber here (annex: «la barra en ámbar desde el 80 %»). */
const USAGE_WARN_PERCENT = 80;

function hubHost(url: string | null | undefined): string | undefined {
  if (url === null || url === undefined || url === "") return undefined;
  try {
    return new URL(url).host;
  } catch {
    return undefined;
  }
}

function KeyValue({
  label,
  children,
  tone,
}: {
  label: string;
  children: ReactNode;
  tone?: "ok" | "warn";
}) {
  return (
    <div className={styles.kv}>
      <span className={styles.kvLabel}>{label}</span>
      <span className={styles.kvValue} data-tone={tone}>
        {children}
      </span>
    </div>
  );
}

/** «Uso de tu plan»: the provider's share with its bar, amber from 80 %, and when it renews. */
function Usage({ usage, nowMs }: { usage: NonNullable<Agent["usage"]>; nowMs: number }) {
  const percent = Math.max(0, Math.min(100, Math.round(usage.used_percent)));
  const tone = percent >= USAGE_WARN_PERCENT ? "warn" : "ok";
  return (
    <div className={styles.kv}>
      <span className={styles.kvLabel}>{copy.usage}</span>
      <span className={styles.usage}>
        <span className={styles.usageTop}>
          <span>{copy.usageLine(percent, usage.window)}</span>
          {usage.resets_at_ms !== undefined && (
            <span className={styles.usageRenews}>{copy.renews(usage.resets_at_ms, nowMs)}</span>
          )}
        </span>
        <span
          aria-label={copy.usage}
          aria-valuemax={100}
          aria-valuemin={0}
          aria-valuenow={percent}
          className={styles.usageBar}
          data-tone={tone}
          role="progressbar"
        >
          <span className={styles.usageFill} style={{ width: `${percent}%` }} />
        </span>
        <span className={styles.usageHint}>{copy.usageHint}</span>
      </span>
    </div>
  );
}

/** «Ver mi plan de …» and «Cerrar sesión», for an assistant with a subscription signed in. */
function AccountActions({ agent, port, send }: { agent: Agent; port?: AgentPort; send: Send }) {
  const vendor = vendorOf(agent.service);
  const plan = agent.service === undefined ? undefined : PLAN_PAGES[agent.service];
  const signingOut = useFeedback();
  const [outcome, setOutcome] = useState<{ ok: boolean; text: string } | null>(null);
  const signOut = () => {
    const work = callAgent(port, { call: "sign_out", agent_id: agent.id }).then((result) => {
      setOutcome(
        result.ok
          ? { ok: true, text: copy.signedOutDone(agent.name) }
          : { ok: false, text: result.text },
      );
      if (!result.ok) throw new Error(result.text);
    });
    signingOut.track(work);
  };
  return (
    <div className={styles.accountActions}>
      <div className={styles.actionsRow}>
        {vendor !== undefined && plan !== undefined && (
          <Button onClick={() => void send({ command: "open_url", url: plan })} size={32}>
            {copy.seePlan(vendor)}
          </Button>
        )}
        <Button feedback={signingOut.feedback} onClick={signOut} size={32} variant="ghost">
          {copy.signOut}
        </Button>
      </div>
      {outcome !== null && (
        <p
          className={outcome.ok ? styles.fieldHint : styles.fieldError}
          role={outcome.ok ? "status" : "alert"}
        >
          {outcome.text}
        </p>
      )}
    </div>
  );
}

/** «Estado»: connected, its session or its limit, or not known yet. */
function stateOf(health: Health, online: boolean): { text: string; tone?: "ok" | "warn" } {
  if (health.kind === "sign_in")
    return {
      text: health.reason === "expired" ? copy.stateExpired : copy.stateSignedOut,
      tone: "warn",
    };
  if (health.kind === "limit") return { text: copy.stateLimit(health.until), tone: "warn" };
  if (health.kind === "unknown" || !online) return { text: copy.stateUnknown };
  return { text: copy.stateConnected, tone: "ok" };
}

/** «lucia.ferrer@nubaria.example · ChatGPT Plus», with what the adapter said of both. */
function accountLine(agent: Agent): string | undefined {
  if (agent.account_email === undefined) return agent.plan;
  return agent.plan === undefined ? agent.account_email : `${agent.account_email} · ${agent.plan}`;
}

/** «Conexión»: the service, the account, its state and the plan's usage. */
export function Connection({
  agent,
  snapshot,
  port,
  send,
  nowMs,
}: {
  agent: Agent;
  snapshot: AppSnapshot;
  port?: AgentPort;
  send: Send;
  nowMs: number;
}) {
  const health = healthOf(agent, nowMs);
  const hermes = agent.service === "hermes";
  const state = stateOf(health, snapshot.settings.hub.connection === "online");
  const host = hubHost(snapshot.settings.hub.url);
  const account = accountLine(agent);
  return (
    <Group label={copy.connection}>
      <List>
        {health.kind === "sign_in" && (
          <div className={styles.connectionNotice}>
            <SignInNotice
              agent={agent}
              agents={agentsOf(snapshot)}
              nowMs={nowMs}
              place="settings"
              port={port}
              send={send}
            />
          </div>
        )}
        {agent.service !== undefined && (
          <KeyValue label={copy.service}>{copy.serviceName(agent.service)}</KeyValue>
        )}
        {hermes && host !== undefined && (
          <KeyValue label={copy.where}>{copy.whereHub(host)}</KeyValue>
        )}
        {account !== undefined && <KeyValue label={copy.account}>{account}</KeyValue>}
        <KeyValue label={copy.state} tone={state.tone}>
          {state.text}
        </KeyValue>
        {agent.usage !== undefined && <Usage nowMs={nowMs} usage={agent.usage} />}
        {agent.account === "signed_in" && <AccountActions agent={agent} port={port} send={send} />}
        {hermes && <p className={styles.connectionNote}>{copy.hubConfigured}</p>}
      </List>
    </Group>
  );
}
