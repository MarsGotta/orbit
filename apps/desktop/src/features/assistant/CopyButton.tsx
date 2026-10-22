import { useEffect, useState } from "react";
import { copyText } from "../../bridge/clipboard";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";
import styles from "./CopyButton.module.css";

// «✓ Copiado» stays long enough to be read after the click, then the button
// says «Copiar» again: the 1.8 s every confirming button keeps (guide 1).
const COPIED_VISIBLE_MS = 1_800;

type CopyState = { kind: "idle" } | { kind: "copied" } | { kind: "failed"; cause: string };

/** Copies to the clipboard and says how it went: «Copiado», or why not. */
export function useCopy() {
  const [state, setState] = useState<CopyState>({ kind: "idle" });
  useEffect(() => {
    if (state.kind !== "copied") return;
    const timer = setTimeout(() => setState({ kind: "idle" }), COPIED_VISIBLE_MS);
    return () => clearTimeout(timer);
  }, [state]);
  const copy = (text: string) =>
    copyText(text).then(
      () => setState({ kind: "copied" }),
      (error: unknown) => setState({ kind: "failed", cause: String(error) }),
    );
  return { copy, state, setState };
}

export function copyStatus(state: CopyState): string | null {
  if (state.kind === "failed") return es.chatActions.copyFailed(state.cause);
  return null;
}

/** A «Copiar» button named for what it copies, that confirms «✓ Copiado». */
export function CopyButton({ label, text }: { label: string; text: () => string }) {
  const { copy, state } = useCopy();
  const failure = copyStatus(state);
  const copied = state.kind === "copied";
  return (
    <>
      <Button
        aria-label={label}
        className={copied ? styles.copied : undefined}
        onClick={() => void copy(text())}
        size={28}
        variant="ghost"
      >
        <span aria-live="polite">{copied ? es.chatActions.copied : es.chatActions.copy}</span>
      </Button>
      {failure !== null && (
        <span className={styles.failure} role="alert">
          {failure}
        </span>
      )}
    </>
  );
}
