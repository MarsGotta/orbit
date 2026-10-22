import type { FormEvent, ReactNode } from "react";
import { Button } from "../../design/Button";
import { TextInput } from "../../design/TextInput";
import { es } from "../../i18n/es";
import styles from "./OnboardingView.module.css";

export function Step({
  children,
  region,
  title,
}: {
  children: ReactNode;
  region?: boolean;
  title: string;
}) {
  return (
    <section aria-label={region === true ? title : undefined} className={styles.step}>
      <h1>{title}</h1>
      {children}
    </section>
  );
}

export function Actions({
  onContinue,
  onSkip,
  skipLabel = es.onboarding.skipStep,
}: {
  onContinue: () => void;
  onSkip: () => void;
  /** What the skip leaves: this step (the default), or the whole first run. */
  skipLabel?: string;
}) {
  return (
    <div className={styles.actions}>
      <Button onClick={onSkip} variant="ghost">
        {skipLabel}
      </Button>
      <Button onClick={onContinue} variant="accent">
        {es.onboarding.continue}
      </Button>
    </div>
  );
}

export function WelcomeStep(props: { onContinue: () => void; onSkip: () => void }) {
  return (
    <Step title={es.onboarding.welcome}>
      <p>{es.onboarding.welcomeBody}</p>
      <Actions {...props} skipLabel={es.onboarding.skipAll} />
    </Step>
  );
}

interface PairProps {
  code: string;
  deviceName: string;
  error: string | null;
  hubUrl: string;
  onCode: (value: string) => void;
  onDeviceName: (value: string) => void;
  onHubUrl: (value: string) => void;
  onPair: () => void;
  onSkip: () => void;
  pending: boolean;
}

export function PairStep(props: PairProps) {
  const submit = (event: FormEvent) => {
    event.preventDefault();
    props.onPair();
  };
  return (
    <form onSubmit={submit}>
      <Step title={es.onboarding.pairTitle}>
        <p>{es.onboarding.pairBody}</p>
        {Boolean(props.error) && (
          <p className={styles.pairError} role="alert">
            {props.error}
          </p>
        )}
        <Field label={es.onboarding.hubUrl} onChange={props.onHubUrl} value={props.hubUrl} />
        <Field label={es.onboarding.code} onChange={props.onCode} value={props.code} />
        <Field
          label={es.onboarding.deviceName}
          onChange={props.onDeviceName}
          value={props.deviceName}
        />
        <div className={styles.actions}>
          <Button onClick={props.onSkip} type="button" variant="ghost">
            {es.onboarding.skipStep}
          </Button>
          <Button disabled={props.pending} type="submit" variant="accent">
            {props.pending ? es.onboarding.pairing : es.onboarding.pair}
          </Button>
        </div>
      </Step>
    </form>
  );
}

export function SavedPairStep({ onContinue }: { onContinue: () => void }) {
  return (
    <Step title={es.onboarding.pairTitle}>
      <p>{es.onboarding.paired}</p>
      <div className={styles.actions}>
        <Button onClick={onContinue} variant="accent">
          {es.onboarding.continue}
        </Button>
      </div>
    </Step>
  );
}

function Field({
  label,
  onChange,
  value,
}: {
  label: string;
  onChange: (value: string) => void;
  value: string;
}) {
  return (
    <label>
      <span>{label}</span>
      <TextInput
        aria-label={label}
        onChange={(event) => onChange(event.target.value)}
        value={value}
        variant="dotted"
      />
    </label>
  );
}

export function ReadyStep({ onComplete }: { onComplete: () => void }) {
  return (
    <Step title={es.onboarding.ready}>
      <p>{es.onboarding.readyBody}</p>
      <Button onClick={onComplete} variant="accent">
        {es.onboarding.start}
      </Button>
    </Step>
  );
}
