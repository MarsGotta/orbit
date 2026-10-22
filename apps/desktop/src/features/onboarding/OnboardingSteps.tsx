import { useId, type FormEvent, type ReactNode } from "react";
import { Field } from "../../design/Field";
import { useSpanish } from "../../i18n/AssistantCopy";
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

export function WelcomeStep() {
  const es = useSpanish();
  const copy = es.onboarding;
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
  onCode: (value: string) => void;
  onDeviceName: (value: string) => void;
  onHubUrl: (value: string) => void;
  onPair: () => void;
  /** The failure's words, or the hub this Mac is paired with. */
  failure: string | null;
  pairedWith: string | null;
}

export function PairStep(props: PairFormProps) {
  const es = useSpanish();
  const copy = es.onboarding;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    props.onPair();
  };
  return (
    <Step lead={copy.pairBody} title={copy.pairTitle}>
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
          autoComplete="one-time-code"
          className={styles.code}
          help={copy.codeHelp}
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
      {props.failure !== null && (
        <p className={`${styles.result} ${styles.failed}`} role="alert">
          {props.failure}
        </p>
      )}
      {props.pairedWith !== null && (
        <p className={`${styles.result} ${styles.paired}`} role="status">
          {copy.paired(props.pairedWith)}
        </p>
      )}
    </Step>
  );
}

export function ReadyStep() {
  const es = useSpanish();
  const copy = es.onboarding;
  return (
    <Step lead={copy.readyBody} title={copy.ready}>
      <ul className={styles.rows}>
        {copy.shortcuts.map((shortcut) => (
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
