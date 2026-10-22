import type { AppView as AppSnapshot } from "../../state/app-view";
import { useId, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { useSpanish } from "../../i18n/AssistantCopy";
import { PermissionsStep } from "./OnboardingAccessSteps";
import { OnboardingFrame, StepAction } from "./OnboardingFrame";
import { PairStep, ReadyStep, WelcomeStep } from "./OnboardingSteps";

const completionKey = "orbit.onboarding.done.local";
const STEPS = 4;

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
  send: (command: AppCommand) => Promise<unknown>;
  snapshot: AppSnapshot;
  startAtPair?: boolean;
}

/** The hub's name as the person reads it: «hub.example», without scheme or path. */
function hostOf(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url.replace(/^[a-z]+:\/\//i, "").replace(/\/.*$/, "");
  }
}

type Pairing = "idle" | "pending" | "accepted" | "refused";

/**
 * Pairing is the one step that can fail on the daemon's side. It waits for
 * the answer: the store's send says whether the order was accepted, and a
 * refusal's words come back as this first run's failure. Only an accepted
 * order with no failure says this Mac is paired. Changing a field starts again.
 */
export type PairingState = ReturnType<typeof usePairing>;

function usePairing(props: Props, hubUrl: string) {
  const [pairing, setPairing] = useState<Pairing>("idle");
  const es = useSpanish();
  const { hub } = props.snapshot.settings;
  const failed = pairing === "refused" || (pairing === "accepted" && props.error !== null);
  const alreadyPaired = hub.paired && props.forcePair !== true && pairing === "idle";
  const paired = (pairing === "accepted" && props.error === null) || alreadyPaired;
  return {
    pending: pairing === "pending",
    failure: failed ? (props.error ?? `${es.failure.title(es.failure.commands.pair)}.`) : null,
    pairedWith: paired ? hostOf(alreadyPaired ? (hub.url ?? hubUrl) : hubUrl) : null,
    reset: () => setPairing("idle"),
    pair: (command: AppCommand) => {
      if (pairing === "pending") return;
      setPairing("pending");
      void props.send(command).then(
        (accepted) => setPairing(accepted === false ? "refused" : "accepted"),
        () => setPairing("refused"),
      );
    },
  };
}

/** The pairing fields; changing one starts the pairing again. */
function usePairForm(props: Props, appName: string) {
  const { hub } = props.snapshot.settings;
  const [hubUrl, setHubUrl] = useState(hub.url ?? import.meta.env.VITE_ORBIT_HUB_URL ?? "");
  const [code, setCode] = useState("");
  const [deviceName, setDeviceName] = useState(hub.device_name ?? appName);
  const pairing = usePairing(props, hubUrl);
  const edited =
    <T,>(set: (value: T) => void) =>
    (value: T) => {
      set(value);
      pairing.reset();
    };
  return {
    pairing,
    fields: {
      code,
      deviceName,
      hubUrl,
      onCode: edited(setCode),
      onDeviceName: edited(setDeviceName),
      onHubUrl: edited(setHubUrl),
      onPair: () =>
        pairing.pair({ command: "pair", hub_url: hubUrl, code, device_name: deviceName }),
    },
  };
}

export function OnboardingView(props: Props) {
  const { onComplete, send, snapshot } = props;
  const es = useSpanish();
  const [step, setStep] = useState(() => (props.startAtPair === true ? 2 : 1));
  const { pairing, fields } = usePairForm(props, es.appName);
  const formId = useId();
  // There is no calendar step. Calendarios is one of the macOS
  // permissions, and granting it is the whole decision. Nor a «Deimos» step:
  // what Deimos may see and do is those same macOS permissions.
  const complete = () => {
    rememberCompletion();
    void send({ command: "onboarding_done" });
    onComplete();
  };
  const body = [
    <WelcomeStep key="welcome" />,
    <PairStep
      {...fields}
      failure={pairing.failure}
      formId={formId}
      key="pair"
      pairedWith={pairing.pairedWith}
    />,
    <PermissionsStep
      error={props.error}
      key="permissions"
      permissions={snapshot.settings.permissions}
      send={send}
    />,
    <ReadyStep key="ready" />,
  ][step - 1];
  return (
    <OnboardingFrame
      action={
        <StepAction
          formId={formId}
          next={() => setStep(Math.min(step + 1, STEPS))}
          onComplete={complete}
          pairing={pairing}
          step={step}
        />
      }
      goTo={setStep}
      onSkipAll={complete}
      step={step}
      steps={STEPS}
    >
      {body}
    </OnboardingFrame>
  );
}
