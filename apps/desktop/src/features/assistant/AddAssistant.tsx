import { useId } from "react";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { useModalFocus } from "../../design/use-modal-focus";
import { assistantsCopy as copy } from "../../i18n/es/assistants";
import { es } from "../../i18n/es";
import type { AppView as AppSnapshot } from "../../state/app-view";
import type { AgentPort } from "../settings/agents/use-agent-config";
import { goTo } from "./assistant-route";
import { ConnectStep, HubStep, KindStep, NameStep, ReadyStep } from "./AddSteps";
import { STEPS, useAddAssistant, type AddFlow } from "./use-add-assistant";
import styles from "./Assistants.module.css";

type Send = (command: AppCommand) => unknown;

/** «Atrás», «Cancelar» and «Continuar» or «Crear asistente»; at the end, «Hablar con …». */
function Footer({
  flow,
  canGoOn,
  shown,
  close,
  onClose,
}: {
  flow: AddFlow;
  canGoOn: boolean;
  shown: string;
  close: () => void;
  onClose: () => void;
}) {
  const { step, setStep, created } = flow;
  if (step === STEPS)
    return (
      <footer className={styles.addFooter}>
        <span className={styles.spacer} />
        <Button onClick={onClose} size={36} variant="ghost">
          {copy.close}
        </Button>
        <Button
          onClick={() => created !== null && goTo({ screen: "agent", agentId: created })}
          size={36}
          variant="accent"
        >
          {copy.talkTo(shown)}
        </Button>
      </footer>
    );
  const back = () => {
    // Back from signing in: the assistant it began is not kept.
    if (step === 2) void flow.discard();
    flow.setRefusal(null);
    setStep(step - 1);
  };
  return (
    <footer className={styles.addFooter}>
      {step > 1 && (
        <Button onClick={back} size={36} variant="ghost">
          {copy.back}
        </Button>
      )}
      <span className={styles.spacer} />
      <Button onClick={close} size={36} variant="ghost">
        {es.controls.cancel}
      </Button>
      <Button
        disabled={!canGoOn || flow.busy === "creating"}
        onClick={() => (step === 3 ? void flow.create() : setStep(step + 1))}
        size={36}
        variant="accent"
      >
        {step === 3 ? (flow.busy === "creating" ? copy.creating : copy.create) : copy.next}
      </Button>
    </footer>
  );
}

/** «Añadir un asistente · paso N de 4», its ×, and the four bars of the steps. */
function AddHeader({
  step,
  titleId,
  onClose,
}: {
  step: number;
  titleId: string;
  onClose: () => void;
}) {
  return (
    <>
      <header className={styles.addHeader}>
        <span className={styles.addStep} id={titleId}>
          {copy.addStep(step)}
        </span>
        <button
          aria-label={es.controls.close}
          className={styles.addClose}
          onClick={onClose}
          type="button"
        >
          <Icon name="close" size={16} />
        </button>
      </header>
      <div aria-hidden="true" className={styles.addBars}>
        {Array.from({ length: STEPS }, (_, index) => (
          <span data-done={index < step || undefined} key={index} />
        ))}
      </div>
    </>
  );
}

/** The step on screen. */
function Step({ flow, offered, shown }: { flow: AddFlow; offered: boolean; shown: string }) {
  if (flow.step === 1) return <KindStep flow={flow} offered={offered} />;
  if (flow.step === 2) return flow.kind === "hermes" ? <HubStep /> : <ConnectStep flow={flow} />;
  if (flow.step === 3) return <NameStep flow={flow} shown={shown} />;
  return <ReadyStep flow={flow} shown={shown} />;
}

/**
 * «Añadir un asistente» (annex 40, 40a–40c), in four steps: what it works
 * with, signing in with the subscription, its name, colour and what it may
 * use, and «… está listo». A window over the view: Esc, its × or a press
 * outside close it, and an assistant begun and not created is removed.
 */
export function AddAssistant({
  snapshot,
  port,
  send,
  onClose,
}: {
  snapshot: AppSnapshot;
  port?: AgentPort;
  send: Send;
  onClose: () => void;
}) {
  const flow = useAddAssistant(snapshot, port, send);
  const panel = useModalFocus(true);
  const titleId = useId();
  const offered =
    flow.kind === "hermes" || (snapshot.settings.addable_agents ?? []).includes(flow.kind);
  const canGoOn =
    flow.step === 1
      ? offered
      : flow.step === 2
        ? flow.kind !== "hermes" && flow.connected
        : flow.problem === null;
  const shown = flow.name.trim() === "" ? (flow.mine?.name ?? "") : flow.name.trim();
  const close = () => {
    if (flow.step < STEPS) void flow.discard();
    onClose();
  };
  return (
    <div
      className={styles.scrim}
      onMouseDown={(event) => event.target === event.currentTarget && close()}
    >
      <section
        aria-labelledby={titleId}
        aria-modal="true"
        className={styles.addDialog}
        onKeyDown={(event) => {
          if (event.key !== "Escape") return;
          event.stopPropagation();
          close();
        }}
        ref={panel}
        role="dialog"
      >
        <AddHeader onClose={close} step={flow.step} titleId={titleId} />
        <div className={styles.addBody}>
          <Step flow={flow} offered={offered} shown={shown} />
          {flow.refusal !== null && (
            <p className={styles.fieldError} role="alert">
              {flow.refusal}
            </p>
          )}
        </div>
        <Footer canGoOn={canGoOn} close={close} flow={flow} onClose={onClose} shown={shown} />
      </section>
    </div>
  );
}
