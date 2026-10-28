import { useCopy } from "../../i18n/CopyProvider";
import { useId, type FormEvent, type ReactNode } from "react";
import type { PermissionSnapshot } from "../../bridge/generated/PermissionSnapshot";
import { Field } from "../../design/Field";

import { readyAgents, type Cast } from "./cast";
import type { PairingState } from "./OnboardingView";
import styles from "./OnboardingView.module.css";

/** One step: a region named by its title, with the one line under it. */
export function Step({
  children,
  lead,
  title,
}: {
  children?: ReactNode;
  lead: string;
  title: string;
}) {
  const heading = useId();
  return (
    <section aria-labelledby={heading} className={styles.step}>
      <h1 className={styles.title} id={heading}>
        {title}
      </h1>
      <p className={styles.lead}>{lead}</p>
      {children}
    </section>
  );
}

/** What Orbit does, the steps to come and that everything stops at once. */
export function WelcomeStep({ cast }: { cast: Cast }) {
  const copy = useCopy().onboarding;
  const plan = copy.plan(
    cast.hubName,
    cast.others.map((agent) => agent.name),
  );
  return (
    <Step lead={copy.welcomeBody} title={copy.welcome}>
      <ul className={styles.points}>
        {copy.welcomePoints.map((point) => (
          <li key={point.title}>
            <strong>{point.title}</strong>
            <span>{point.body}</span>
          </li>
        ))}
      </ul>
      <ol aria-label={copy.planLabel} className={styles.plan}>
        {plan.map((item, index) => (
          <li className={styles.planRow} key={item.title}>
            <span aria-hidden="true" className={styles.planNumber}>
              {index + 2}
            </span>
            <span className={styles.planTitle}>{item.title}</span>
            <span className={styles.planBody}>{item.body}</span>
          </li>
        ))}
      </ol>
      <p className={styles.quiet}>{copy.welcomeControl}</p>
    </Step>
  );
}

/** What the pairing form needs; its result is said below the fields. */
export interface PairFormProps {
  code: string;
  deviceName: string;
  hubUrl: string;
  formId: string;
  hubName: string;
  onCode: (value: string) => void;
  onDeviceName: (value: string) => void;
  onHubUrl: (value: string) => void;
  onPair: () => void;
  pairing: PairingState;
}

export function PairStep(props: PairFormProps) {
  const copy = useCopy().onboarding;
  const { pairing } = props;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    props.onPair();
  };
  return (
    <Step lead={copy.pairBody(props.hubName)} title={copy.pairTitle}>
      <form className={styles.form} id={props.formId} onSubmit={submit}>
        <Field
          autoComplete="url"
          label={copy.hubUrl}
          onChange={(event) => props.onHubUrl(event.currentTarget.value)}
          placeholder={copy.hubUrlPlaceholder}
          size={40}
          type="url"
          value={props.hubUrl}
        />
        <Field
          aria-invalid={pairing.shortCode ? true : undefined}
          autoComplete="one-time-code"
          className={styles.code}
          help={pairing.shortCode ? copy.codeDigits(props.code.length) : copy.codeHelp}
          inputMode="numeric"
          label={copy.code}
          maxLength={6}
          // Only the six digits: a pasted «482 913» or «482-913» keeps them.
          onChange={(event) =>
            props.onCode(event.currentTarget.value.replace(/\D/g, "").slice(0, 6))
          }
          placeholder="000000"
          value={props.code}
        />
        <Field
          label={copy.deviceName}
          onChange={(event) => props.onDeviceName(event.currentTarget.value)}
          size={40}
          value={props.deviceName}
        />
      </form>
      {pairing.failed && (
        <div className={styles.failure} role="alert">
          <span aria-hidden="true" className={styles.failureDot} />
          <span className={styles.failureText}>
            <strong>{copy.pairFailed}</strong>
            <span>{pairing.refusal ?? copy.pairFailedHint}</span>
          </span>
        </div>
      )}
      {pairing.pairedWith !== null && (
        <p className={`${styles.result} ${styles.paired}`} role="status">
          {copy.paired(pairing.pairedWith, props.hubName)}
        </p>
      )}
    </Step>
  );
}

interface ReadyProps {
  cast: Cast;
  deviceName: string | null;
  host: string;
  paired: boolean;
  permissions: PermissionSnapshot;
}

/** What is set up, in green, and what is missing, in amber, with what to do about it. */
function useSummary({ cast, deviceName, host, paired, permissions }: ReadyProps) {
  const permissionRows = useCopy().extras.permissionRows;
  const copy = useCopy().onboarding;
  const missing = permissionRows
    .filter(([key]) => permissions[key] !== "granted")
    .map(([, permission]) => permission.label);
  const granted = permissionRows.length - missing.length;
  const ready = readyAgents(cast, paired).map((agent) =>
    agent.id === cast.primaryId ? copy.asPrimary(agent.name) : agent.name,
  );
  return [
    {
      key: copy.summaryHub,
      ok: paired,
      value: paired ? copy.hubPaired(host, deviceName) : copy.hubUnpaired,
    },
    {
      key: copy.summaryPermissions,
      ok: missing.length === 0,
      value: missing.length === 0 ? copy.allGranted : copy.someGranted(granted, missing),
    },
    {
      key: copy.summaryAssistants,
      ok: ready.length > 0,
      value: ready.length > 0 ? copy.assistantsReady(ready) : copy.noAssistants,
    },
  ];
}

export function ReadyStep(props: ReadyProps) {
  const copy = useCopy().onboarding;
  const summary = useSummary(props);
  const shortcutsTitle = useId();
  return (
    <Step lead={copy.readyBody} title={copy.ready}>
      <ul aria-label={copy.summaryLabel} className={styles.rows}>
        {summary.map((row) => (
          <li className={styles.summaryRow} data-tone={row.ok ? "ok" : "warn"} key={row.key}>
            <span aria-hidden="true" className={styles.summaryDot} />
            <span className={styles.summaryKey}>{row.key}</span>
            <span className={styles.summaryValue}>{row.value}</span>
          </li>
        ))}
      </ul>
      <h2 className={styles.subtitle} id={shortcutsTitle}>
        {copy.shortcutsTitle}
      </h2>
      <ul aria-labelledby={shortcutsTitle} className={styles.rows}>
        {copy.shortcuts(props.cast.primaryName).map((shortcut) => (
          <li className={styles.shortcutRow} key={shortcut.keys}>
            <kbd className={styles.keys}>{shortcut.keys}</kbd>
            <span className={styles.rowText}>
              <strong>{shortcut.title}</strong>
              <span>{shortcut.body}</span>
            </span>
          </li>
        ))}
      </ul>
    </Step>
  );
}
