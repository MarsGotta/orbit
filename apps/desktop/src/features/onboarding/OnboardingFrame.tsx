import { useEffect, useRef, type ReactNode } from "react";
import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { useSpanish } from "../../i18n/AssistantCopy";
import type { PairingState } from "./OnboardingView";
import styles from "./OnboardingView.module.css";

/**
 * While the first run covers the window, what it covers cannot be reached
 * by keyboard or read aloud. The corner notices stay: a failure is said.
 */
function useCoveredInert() {
  const overlay = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const own = overlay.current;
    const covered = Array.from(own?.parentElement?.children ?? []).filter(
      (element) =>
        element !== own && element.id !== "orbit-notices" && !element.hasAttribute("inert"),
    );
    for (const element of covered) {
      element.setAttribute("inert", "");
      element.setAttribute("aria-hidden", "true");
    }
    return () => {
      for (const element of covered) {
        element.removeAttribute("inert");
        element.removeAttribute("aria-hidden");
      }
    };
  }, []);
  return overlay;
}

interface FrameProps {
  /** The step's main action, at the right of «Atrás». */
  action: ReactNode;
  children: ReactNode;
  goTo: (step: number) => void;
  onSkipAll: () => void;
  step: number;
  steps: number;
}

/**
 * Around every step: Orbit's name and «Configurarlo más tarde» above,
 * «Paso N de 4» with its bars (a bar goes back to a step already seen),
 * and «Atrás» below.
 */
export function OnboardingFrame({ action, children, goTo, onSkipAll, step, steps }: FrameProps) {
  const es = useSpanish();
  const copy = es.onboarding;
  const mac = navigator.userAgent.includes("Mac");
  const overlay = useCoveredInert();
  return (
    <div className={styles.overlay} ref={overlay}>
      <header className={`${styles.header} ${mac ? styles.mac : ""}`}>
        <span className={styles.brand}>
          <Icon name="logo" size={20} />
          <span>{es.appName}</span>
        </span>
        {step < steps && (
          <Button onClick={onSkipAll} size={32} variant="ghost">
            {copy.skipAll}
          </Button>
        )}
      </header>
      <main className={styles.column}>
        <div className={styles.progress}>{copy.stepOf(step, steps)}</div>
        <ol aria-label={copy.stepsLabel} className={styles.bars}>
          {copy.stepNames.map((name, index) => {
            const number = index + 1;
            return (
              <li key={name}>
                <button
                  aria-current={number === step ? "step" : undefined}
                  aria-label={copy.stepLabel(number, name)}
                  className={styles.bar}
                  data-done={number <= step}
                  disabled={number > step}
                  onClick={() => goTo(number)}
                  type="button"
                />
              </li>
            );
          })}
        </ol>
        <div className={styles.body}>{children}</div>
        <div className={styles.footer}>
          {step > 1 && (
            <Button onClick={() => goTo(step - 1)} size={40} variant="ghost">
              {copy.back}
            </Button>
          )}
          <span className={styles.spacer} />
          {action}
        </div>
      </main>
    </div>
  );
}

interface ActionProps {
  formId: string;
  next: () => void;
  onComplete: () => void;
  pairing: PairingState;
  step: number;
}

/** «Empezar», «Emparejar» (or «Hacerlo después»), «Continuar» or «Empezar a usar Orbit». */
export function StepAction({ formId, next, onComplete, pairing, step }: ActionProps) {
  const copy = useSpanish().onboarding;
  if (step === 1)
    return (
      <Button onClick={next} size={40} variant="accent">
        {copy.begin}
      </Button>
    );
  if (step === 2 && pairing.pairedWith === null)
    return (
      <>
        <Button onClick={next} size={40} variant="ghost">
          {copy.skipStep}
        </Button>
        <Button
          feedback={pairing.pending ? "pending" : "idle"}
          form={formId}
          pendingLabel={copy.pairing}
          size={40}
          type="submit"
          variant="accent"
        >
          {copy.pair}
        </Button>
      </>
    );
  if (step === 4)
    return (
      <Button onClick={onComplete} size={40} variant="accent">
        {copy.start}
      </Button>
    );
  return (
    <Button onClick={next} size={40} variant="accent">
      {copy.continue}
    </Button>
  );
}
