import { useEffect } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { Agent } from "../../bridge/generated/Agent";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { Button } from "../../design/Button";
import { Tooltip } from "../../design/Tooltip";
import { assistantsCopy as copy, vendorOf } from "../../i18n/es/assistants";
import { assistantHash } from "./assistant-route";
import { colorOf, healthOf, type Health } from "./assistants";
import { handingOver, stopWaiting, useHandovers, waitFor } from "./handover";
import styles from "./AssistantPages.module.css";

type Send = (command: AppCommand) => unknown;
type Offer = { agent: Agent; health: Health };

/** The other assistants to go on with: the primary first, then the rest, each as it can. */
function others(
  agent: Agent,
  agents: readonly Agent[],
  primaryId: string | undefined,
  nowMs: number,
): Offer[] {
  const rest = agents.filter((each) => each.id !== agent.id);
  rest.sort((a, b) => Number(b.id === primaryId) - Number(a.id === primaryId));
  return rest
    .map((each) => ({ agent: each, health: healthOf(each, nowMs) }))
    .filter(({ health }) => health.kind === "ok" || health.kind === "sign_in");
}

/**
 * «Esperar a las …»: once the plan is back and the hub lifted the limit,
 * the question goes again, once. The clock is read here, since what wakes
 * this up may be the hub lifting the limit between two ticks of the view.
 */
function useWaitedQuestion(agent: Agent, sessionId: string | null, send: Send, nowMs: number) {
  const waiting = useHandovers((state) =>
    sessionId === null ? undefined : state.waiting[sessionId],
  );
  const clock = Math.max(nowMs, Date.now());
  const back =
    waiting !== undefined && clock >= waiting.until && healthOf(agent, clock).kind !== "limit";
  useEffect(() => {
    if (!back || sessionId === null || waiting === undefined) return;
    stopWaiting(sessionId);
    void send({ command: "chat", text: waiting.question });
  }, [back, send, sessionId, waiting]);
  return waiting;
}

/** «Seguir con …» for one that can answer; one that needs its session leads there first. */
function OfferButton({
  offer,
  agents,
  accent,
  onHandOver,
}: {
  offer: Offer;
  agents: readonly Agent[];
  accent: boolean;
  onHandOver: (to: Agent) => void;
}) {
  const { agent, health } = offer;
  const avatar = <AssistantAvatar color={colorOf(agent, agents)} name={agent.name} size={20} />;
  if (health.kind !== "sign_in")
    return (
      <Button onClick={() => onHandOver(agent)} size={32} variant={accent ? "accent" : "outline"}>
        {avatar}
        {copy.continueWith(agent.name)}
      </Button>
    );
  return (
    <Tooltip text={copy.signInFirstTip(health.vendor)}>
      <a
        className={styles.outlineLink}
        href={assistantHash({ screen: "agent", agentId: agent.id })}
      >
        {avatar}
        {copy.signInFirst(agent.name)}
      </a>
    </Tooltip>
  );
}

/** «Phobos ha llegado al límite de tu plan de ChatGPT». */
function limitTitle(agent: Agent): string {
  const vendor = vendorOf(agent.service);
  return vendor === undefined
    ? copy.limitTitlePlan(agent.name)
    : copy.limitTitle(agent.name, vendor);
}

/** «Te avisamos cuando … vuelva a estar disponible», and «Seguir con otro» to stop waiting. */
function WaitLine({
  name,
  sessionId,
  until,
}: {
  name: string;
  sessionId: string | null;
  until: number;
}) {
  return (
    <p className={styles.waitLine} role="status">
      {copy.waitingFor(name, until)}{" "}
      <button
        className={styles.linkButton}
        onClick={() => sessionId !== null && stopWaiting(sessionId)}
        type="button"
      >
        {copy.continueWithOther}
      </button>
    </p>
  );
}

/**
 * A turn stopped at its assistant's plan's limit (annex 41): the question
 * is kept, «Seguir con …» hands the conversation to another assistant that
 * reads it, one that needs its session first leads there, and «Esperar a
 * las …» sends the question again once the plan is back.
 */
export function LimitCard(props: {
  agent: Agent;
  agents: readonly Agent[];
  primaryId?: string;
  sessionId: string | null;
  question: string | null;
  until?: number;
  /** The hub hands conversations over (it sends `addable_agents`). */
  canHandOver: boolean;
  send: Send;
  nowMs: number;
}) {
  const { agent, agents, primaryId, sessionId, question, until, canHandOver, send, nowMs } = props;
  const waiting = useWaitedQuestion(agent, sessionId, send, nowMs);
  if (waiting !== undefined)
    return <WaitLine name={agent.name} sessionId={sessionId} until={waiting.until} />;
  const offers = canHandOver && sessionId !== null ? others(agent, agents, primaryId, nowMs) : [];
  const primary = offers.find(
    (offer) => offer.agent.id === primaryId && offer.health.kind === "ok",
  );
  const handOver = (to: Agent) => {
    if (sessionId === null) return;
    handingOver(sessionId, agent.id, to.id);
    void send({ command: "handover", session_id: sessionId, agent_id: to.id, text: "" });
  };
  return (
    <div className={styles.limit} role="alert">
      <span aria-hidden="true" className={styles.limitDot} />
      <div className={styles.limitBody}>
        <h3 className={styles.limitTitle}>{limitTitle(agent)}</h3>
        <p className={styles.limitText}>{copy.limitText(until, nowMs)}</p>
        <div className={styles.limitActions}>
          {offers.map((offer) => (
            <OfferButton
              accent={offer === primary}
              agents={agents}
              key={offer.agent.id}
              offer={offer}
              onHandOver={handOver}
            />
          ))}
          {until !== undefined && question !== null && sessionId !== null && (
            <Button
              onClick={() => waitFor(sessionId, { agentId: agent.id, until, question })}
              size={32}
              variant="ghost"
            >
              {copy.waitUntil(until)}
            </Button>
          )}
        </div>
        {primary !== undefined && (
          <p className={styles.limitWhy}>{copy.whyPrimary(primary.agent.name)}</p>
        )}
      </div>
    </div>
  );
}
