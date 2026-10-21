import type { AppView as AppSnapshot } from "../../state/app-view";
import { useEffect, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { Card } from "../../design/Card";
import { es } from "../../i18n/es";
import { PermissionsStep } from "./OnboardingAccessSteps";
import { PairStep, ReadyStep, SavedPairStep, WelcomeStep } from "./OnboardingSteps";
import styles from "./OnboardingView.module.css";

const completionKey = "orbit.onboarding.done.local";

export function onboardingWasCompleted(): boolean {
  try {
    return window.localStorage.getItem(completionKey) === "true";
  } catch {
    return false;
  }
}

function rememberCompletion(): void {
  try {
    window.localStorage.setItem(completionKey, "true");
  } catch {
    // The current view keeps completion in memory when webview storage is unavailable.
  }
}

interface Props {
  error: string | null;
  forcePair?: boolean;
  onComplete: () => void;
  send: (command: AppCommand) => Promise<void>;
  snapshot: AppSnapshot;
  startAtPair?: boolean;
}

function useContentInert(step: number): void {
  useEffect(() => {
    const content = document.querySelector<HTMLElement>("[data-app-content]");
    if (step > 0) {
      content?.setAttribute("aria-hidden", "true");
      content?.setAttribute("inert", "");
    }
    return () => {
      content?.removeAttribute("aria-hidden");
      content?.removeAttribute("inert");
    };
  }, [step]);
}

interface HubStepProps {
  code: string;
  deviceName: string;
  error: string | null;
  forcePair: boolean;
  hub: AppSnapshot["settings"]["hub"];
  hubUrl: string;
  next: () => void;
  send: Props["send"];
  setCode: (value: string) => void;
  setDeviceName: (value: string) => void;
  setHubUrl: (value: string) => void;
}

/**
 * Pairing is the one step that can fail on the daemon's side. It waits for the
 * answer, moves on only when the store reports no failure, and hands the cause
 * to the step when it does; advancing regardless left people believing this Mac
 * was paired when it never was.
 */
function HubStep(props: HubStepProps) {
  const [pending, setPending] = useState(false);
  const [attempted, setAttempted] = useState(false);
  const { error, next, send } = props;
  useEffect(() => {
    if (!attempted || pending) return;
    setAttempted(false);
    if (error === null) next();
  }, [attempted, error, next, pending]);
  if (props.hub.paired && !props.forcePair) {
    return <SavedPairStep onContinue={props.next} />;
  }
  return (
    <PairStep
      code={props.code}
      deviceName={props.deviceName}
      error={attempted || pending ? null : error}
      hubUrl={props.hubUrl}
      onCode={props.setCode}
      onDeviceName={props.setDeviceName}
      onHubUrl={props.setHubUrl}
      onPair={() => {
        setPending(true);
        setAttempted(true);
        void send({
          command: "pair",
          hub_url: props.hubUrl,
          code: props.code,
          device_name: props.deviceName,
        }).finally(() => setPending(false));
      }}
      onSkip={props.next}
      pending={pending}
    />
  );
}

export function OnboardingView({
  error,
  forcePair = false,
  onComplete,
  send,
  snapshot,
  startAtPair = false,
}: Props) {
  const [step, setStep] = useState(() => (startAtPair ? 1 : 0));
  const [hubUrl, setHubUrl] = useState(
    snapshot.settings.hub.url ?? import.meta.env.VITE_ORBIT_HUB_URL ?? "",
  );
  const [code, setCode] = useState("");
  const [deviceName, setDeviceName] = useState(snapshot.settings.hub.device_name ?? es.appName);
  // There is no calendar step. Calendarios is one of the macOS
  // permissions in the step before, and granting it is the whole decision.
  // Nor a «Deimos» step. What Deimos may see and do is those same
  // macOS permissions; the first run is pairing and granting them.
  const next = () => setStep((current) => Math.min(current + 1, 3));
  useContentInert(step);
  const complete = () => {
    rememberCompletion();
    send({ command: "onboarding_done" });
    onComplete();
  };

  const steps = [
    <WelcomeStep key="welcome" onContinue={next} onSkip={complete} />,
    <HubStep
      code={code}
      deviceName={deviceName}
      error={error}
      forcePair={forcePair}
      hub={snapshot.settings.hub}
      hubUrl={hubUrl}
      key="pair"
      next={next}
      send={send}
      setCode={setCode}
      setDeviceName={setDeviceName}
      setHubUrl={setHubUrl}
    />,
    <PermissionsStep
      key="permissions"
      onContinue={next}
      permissions={snapshot.settings.permissions}
      send={send}
    />,
    <ReadyStep key="ready" onComplete={complete} />,
  ];
  return (
    <div className={styles.overlay}>
      <Card>{steps[step]}</Card>
    </div>
  );
}
