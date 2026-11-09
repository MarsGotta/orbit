import { useCopy } from "../../i18n/CopyProvider";
import { useId, type KeyboardEvent } from "react";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { Button } from "../../design/Button";
import { moveWithArrows } from "../../design/roving";
import { Spinner } from "../../design/Spinner";
import { Switch } from "../../design/Switch";

import { NAME_MAX } from "./assistants";
import { ColorChoice } from "./ProfileFields";
import { sharedAccount, SignInWaiting } from "./SignInNotice";
import { KINDS, type AddFlow, type Kind } from "./use-add-assistant";
import styles from "./AssistantPages.module.css";

/** Step 1: «¿Con qué va a funcionar?». */
export function KindStep({ flow, offered }: { flow: AddFlow; offered: boolean }) {
  const copy = useCopy().extras.assistantsCopy;
  const { kind, setKind } = flow;
  return (
    <>
      <h2 className={styles.addTitle}>{copy.kindTitle}</h2>
      <p className={styles.addLead}>{copy.kindLead}</p>
      <div aria-label={copy.kindLabel} className={styles.kinds} role="radiogroup">
        {KINDS.map((each: Kind) => (
          <button
            aria-checked={each === kind}
            className={styles.kind}
            key={each}
            onClick={() => setKind(each)}
            onKeyDown={(event: KeyboardEvent<HTMLElement>) =>
              moveWithArrows(event, KINDS, kind, setKind)
            }
            role="radio"
            tabIndex={each === kind ? 0 : -1}
            type="button"
          >
            <span aria-hidden="true" className={styles.radioDot} />
            <span className={styles.kindText}>
              <span className={styles.kindTitle}>{copy.kinds[each]?.title}</span>
              <span className={styles.kindHelp}>{copy.kinds[each]?.text}</span>
            </span>
            <span className={styles.kindTag}>{copy.kinds[each]?.tag}</span>
          </button>
        ))}
      </div>
      {!offered && (
        <p className={styles.fieldHint} role="status">
          {copy.notOffered}
        </p>
      )}
    </>
  );
}

/** Step 2 for Hermes: it is not added from Orbit. */
export function HubStep() {
  const copy = useCopy().extras.assistantsCopy;
  return (
    <>
      <h2 className={styles.addTitle}>{copy.hubKindTitle}</h2>
      <p className={styles.addLead}>{copy.hubKindLead}</p>
      <div className={styles.addBox}>
        <p className={styles.addBoxTitle}>{copy.askHub}</p>
        <p className={styles.addBoxText}>{copy.askHubText}</p>
      </div>
    </>
  );
}

/** What the box of step 2 holds: the button, the hub preparing it, the wait, or «✓ Conectado». */
function ConnectBox({ flow }: { flow: AddFlow }) {
  const copy = useCopy().extras.assistantsCopy;
  const { mine, agents, signIn, vendor } = flow;
  const { view } = signIn;
  if (flow.connected && mine !== undefined) {
    const other = sharedAccount(mine, agents);
    return (
      <div role="status">
        <p className={styles.ok}>{copy.connected}</p>
        <p className={styles.addBoxText}>{copy.connectedAs(mine.account_email, mine.plan)}</p>
        {other !== undefined && <p className={styles.addBoxText}>{copy.sameAccount(other.name)}</p>}
      </div>
    );
  }
  const waiting = view.state === "waiting" || view.state === "starting";
  if (flow.busy === "preparing" || waiting)
    return (
      <div role="status">
        <p className={styles.waitingLine}>
          <Spinner size={14} />
          {flow.busy === "preparing" ? copy.preparing : copy.waitingTitle}
        </p>
        {waiting && (
          <span className={styles.addBoxText}>
            <SignInWaiting
              onCancel={() => void signIn.cancel()}
              onCode={(code) => void signIn.submitCode(code)}
              vendor={vendor}
              view={view}
            />
          </span>
        )}
      </div>
    );
  return (
    <>
      <Button onClick={() => void flow.begin()} size={40} variant="accent">
        {copy.signIn(vendor)}
      </Button>
      <p className={styles.addBoxText}>
        {view.state === "idle" && view.ended !== undefined ? view.ended : copy.connectHint}
      </p>
    </>
  );
}

/** Step 2 with a subscription: «Conecta tu cuenta de …». */
export function ConnectStep({ flow }: { flow: AddFlow }) {
  const copy = useCopy().extras.assistantsCopy;
  return (
    <>
      <h2 className={styles.addTitle}>{copy.connectTitle(flow.vendor)}</h2>
      <p className={styles.addLead}>{copy.connectLead}</p>
      <div className={styles.addBox}>
        <ConnectBox flow={flow} />
      </div>
    </>
  );
}

/** Step 3: «Ponle nombre», its colour and what it may use. */
export function NameStep({ flow, shown }: { flow: AddFlow; shown: string }) {
  const copy = useCopy().extras.assistantsCopy;
  const accessGroups = useCopy().extras.accessGroups;
  const { name, setName, color, setColor, access, setAccess, problem } = flow;
  const id = useId();
  const hint =
    problem === "empty"
      ? copy.nameEmpty
      : problem === "taken"
        ? copy.nameTaken
        : copy.nameLong(NAME_MAX);
  return (
    <>
      <h2 className={styles.addTitle}>{copy.nameTitle}</h2>
      <p className={styles.addLead}>{copy.nameLead(shown)}</p>
      <div className={styles.nameRow}>
        <AssistantAvatar color={color} name={shown === "" ? "?" : shown} size={40} />
        <div className={styles.nameField}>
          <label className={styles.fieldLabel} htmlFor={id}>
            {copy.name}
          </label>
          <input
            aria-describedby={`${id}-hint`}
            aria-invalid={problem !== null}
            className={styles.input}
            data-invalid={problem !== null || undefined}
            id={id}
            maxLength={NAME_MAX}
            onChange={(event) => setName(event.target.value)}
            type="text"
            value={name}
          />
          {problem !== null && (
            <span className={styles.fieldError} id={`${id}-hint`}>
              {hint}
            </span>
          )}
        </div>
        <ColorChoice label={copy.color} onChange={setColor} value={color} />
      </div>
      <p className={styles.accessTitle}>{copy.accessTitle}</p>
      <div className={styles.accessList}>
        {accessGroups.map(({ group, label, description }) => (
          <div className={styles.accessRow} key={group}>
            <span className={styles.accessText}>
              <span className={styles.accessName}>{label}</span>
              <span className={styles.accessHelp}>{description}</span>
            </span>
            <Switch
              checked={access[group]}
              label={label}
              onCheckedChange={(checked) => setAccess({ ...access, [group]: checked })}
            />
          </div>
        ))}
      </div>
    </>
  );
}

/** Step 4: «… está listo». */
export function ReadyStep({ flow, shown }: { flow: AddFlow; shown: string }) {
  const copy = useCopy().extras.assistantsCopy;
  const line = copy.serviceFull(flow.kind);
  return (
    <div className={styles.ready}>
      <AssistantAvatar color={flow.color} name={shown} size={56} />
      <h2 className={styles.readyTitle}>{copy.readyTitle(shown)}</h2>
      <p className={styles.addLead}>{copy.readyText(shown)}</p>
      {line !== undefined && <p className={styles.fieldHint}>{copy.worksWith(line)}.</p>}
    </div>
  );
}
