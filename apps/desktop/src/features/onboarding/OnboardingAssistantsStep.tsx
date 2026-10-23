import { useId, type ReactNode } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { Button } from "../../design/Button";
import { Loading } from "../../design/Spinner";
import { useSpanish } from "../../i18n/AssistantCopy";
import { assistantsCopy, vendorOf } from "../../i18n/es/assistants";
import { colorOf } from "../assistant/assistants";
import { SignInWaiting } from "../assistant/SignInNotice";
import { useSignIn } from "../assistant/use-sign-in";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { usePrimary } from "../settings/use-primary";
import { readyAgents, type Cast } from "./cast";
import { Step } from "./OnboardingSteps";
import styles from "./OnboardingView.module.css";

type Send = (command: AppCommand) => Promise<unknown>;

/** The hub's own assistant: ready once this Mac is paired, or «Emparejar el hub». */
function HubState({ paired, onPair }: { paired: boolean; onPair: () => void }) {
  const copy = useSpanish().onboarding;
  if (paired) return <span className={styles.granted}>{copy.readyWithHub}</span>;
  return (
    <span className={styles.rowActions}>
      <span className={styles.missing}>
        <span aria-hidden="true" className={styles.missingDot} />
        {copy.needsHub}
      </span>
      <Button onClick={onPair} size={28} variant="outline">
        {copy.pairHub}
      </Button>
    </span>
  );
}

/**
 * One with a subscription: connected, or «Iniciar sesión con …» through the
 * hub (part of the assistants' own flow: the hub starts the vendor's tool,
 * the Mac opens the browser and never sees the token). While the browser is
 * open the row waits, with Codex's code or Claude's to paste and «Cancelar».
 */
function SubscriptionRow({
  agent,
  cast,
  port,
  send,
}: {
  agent: Agent;
  cast: Cast;
  port?: AgentPort;
  send: Send;
}) {
  const copy = useSpanish().onboarding;
  const vendor = vendorOf(agent.service) ?? assistantsCopy.serviceName(agent.service) ?? agent.name;
  const signedIn = agent.account === "signed_in";
  const signIn = useSignIn({ port, send, vendor, name: agent.name, signedIn });
  const { view } = signIn;
  const waiting = view.state === "starting" || view.state === "waiting";
  const needsSignIn = agent.account === "expired" || agent.account === "signed_out";
  const account = copy.account(agent.account_email, agent.plan);
  return (
    <AssistantRow agent={agent} cast={cast}>
      {{
        detail:
          signedIn && account !== "" ? <span className={styles.account}>{account}</span> : null,
        state: signedIn ? (
          <span className={styles.granted}>{copy.connected}</span>
        ) : waiting ? (
          <Loading size={12}>{copy.waitingForBrowser}</Loading>
        ) : needsSignIn ? (
          <Button onClick={() => void signIn.start(agent.id)} size={32} variant="outline">
            {copy.signInWith(vendor)}
          </Button>
        ) : null,
        below: waiting ? (
          <span className={styles.signInWait} role="status">
            <SignInWaiting
              onCancel={() => void signIn.cancel()}
              onCode={(code) => void signIn.submitCode(code)}
              vendor={vendor}
              view={view}
            />
          </span>
        ) : view.state === "idle" && view.ended !== undefined && !signedIn ? (
          <span className={styles.signInEnded} role={view.failed === true ? "alert" : "status"}>
            {view.ended}
          </span>
        ) : null,
      }}
    </AssistantRow>
  );
}

/** A row of «Tus asistentes»: avatar, name, service and what it needs. */
function AssistantRow({
  agent,
  cast,
  children,
}: {
  agent: Pick<Agent, "id" | "name" | "service">;
  cast: Cast;
  children: { detail?: ReactNode; state: ReactNode; below?: ReactNode };
}) {
  const service = assistantsCopy.serviceFull(agent.service ?? "hermes");
  const known = cast.agents.find((each) => each.id === agent.id);
  return (
    <li className={styles.assistantRow}>
      <span className={styles.assistantLine}>
        <AssistantAvatar
          color={known === undefined ? 1 : colorOf(known, cast.agents)}
          name={agent.name}
          size={36}
        />
        <span className={styles.rowText}>
          <strong>{agent.name}</strong>
          {service !== undefined && <span>{service}</span>}
          {children.detail}
        </span>
        {children.state}
      </span>
      {children.below}
    </li>
  );
}

/** «Asistente principal»: one of those ready, kept by the hub. */
function PrimaryChoice({ cast, paired, port }: { cast: Cast; paired: boolean; port?: AgentPort }) {
  const copy = useSpanish().onboarding;
  const titleId = useId();
  const ready = readyAgents(cast, paired).filter((agent) => agent.id !== "");
  const primary = usePrimary(port, cast.agents, cast.primaryId);
  const other = ready.find((agent) => agent.id !== primary.current)?.name;
  return (
    <>
      <h2 className={styles.subtitle} id={titleId}>
        {copy.primary}
      </h2>
      {ready.length === 0 ? (
        <p className={styles.note}>{copy.noPrimary}</p>
      ) : (
        <>
          <p className={styles.subtitleHelp}>{copy.primaryHelp(other)}</p>
          <div aria-labelledby={titleId} className={styles.primaries} role="radiogroup">
            {ready.map((agent) => {
              const known = cast.agents.find((each) => each.id === agent.id);
              const checked = agent.id === primary.current;
              return (
                <button
                  aria-checked={checked}
                  aria-label={agent.name}
                  className={styles.primary}
                  disabled={primary.saving}
                  key={agent.id}
                  onClick={() => void primary.choose(agent.id)}
                  role="radio"
                  type="button"
                >
                  <AssistantAvatar
                    color={known === undefined ? 1 : colorOf(known, cast.agents)}
                    name={agent.name}
                    size={20}
                  />
                  {agent.name}
                </button>
              );
            })}
          </div>
          {primary.refusal !== null && (
            <p className={`${styles.result} ${styles.failed}`} role="alert">
              {primary.refusal}
            </p>
          )}
        </>
      )}
    </>
  );
}

/**
 * Step 4, «Tus asistentes»: each one's state, signing in the ones with a
 * subscription, and the primary among those ready. Nothing here blocks:
 * «Continuar» goes on with any of them missing.
 */
export function AssistantsStep({
  cast,
  onPair,
  paired,
  port,
  send,
}: {
  cast: Cast;
  onPair: () => void;
  paired: boolean;
  port?: AgentPort;
  send: Send;
}) {
  const copy = useSpanish().onboarding;
  const hubAgents = cast.agents.filter((agent) => agent.account === undefined);
  // Before the hub has said its agents, its own assistant by the name it last had.
  const hubRows = hubAgents.length > 0 ? hubAgents : [{ id: "", name: cast.hubName }];
  return (
    <Step lead={copy.assistantsBody(cast.hubName)} title={copy.assistants}>
      <ul aria-label={copy.assistants} className={styles.rows}>
        {hubRows.map((agent) => (
          <AssistantRow agent={agent} cast={cast} key={agent.id}>
            {{ state: <HubState onPair={onPair} paired={paired} /> }}
          </AssistantRow>
        ))}
        {cast.others.map((agent) => (
          <SubscriptionRow agent={agent} cast={cast} key={agent.id} port={port} send={send} />
        ))}
      </ul>
      <PrimaryChoice cast={cast} paired={paired} port={port} />
      <p className={styles.note}>{copy.assistantsLater}</p>
    </Step>
  );
}
