import { getCopy } from "../../i18n";
import { useCopy } from "../../i18n/CopyProvider";
import { useEffect, useState } from "react";
import { copyText } from "../../bridge/clipboard";
import { IconButton } from "../../design/IconButton";

import styles from "./CopyButton.module.css";

// «✓ Copiado» stays long enough to be read after the click, then the button
// says «Copiar» again: the 1.8 s every confirming button keeps (guide 1).
const COPIED_VISIBLE_MS = 1_800;

type CopyState = { kind: "idle" } | { kind: "copied" } | { kind: "failed"; cause: string };

/** Copies to the clipboard and says how it went: «Copiado», or why not. */
export function useClipboardCopy() {
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
  if (state.kind === "failed") return getCopy().chatActions.copyFailed(state.cause);
  return null;
}

/**
 * Copiar as an icon: its tooltip says what it does (`tip`, «Copiar» by
 * default) and its name what it copies (`label`, «Copiar respuesta»). Once
 * copied it turns to ✓ with «Copiado»; a failure is said beside it.
 */
export function CopyButton({
  label,
  tip,
  text,
}: {
  label: string;
  tip?: string;
  text: () => string;
}) {
  const es = useCopy();
  tip ??= es.chatActions.copy;
  const { copy, state } = useClipboardCopy();
  const failure = copyStatus(state);
  return (
    <>
      <IconButton
        aria-label={label}
        doneLabel={es.chatActions.copiedTip}
        feedback={state.kind === "copied" ? "done" : "idle"}
        icon="copy"
        label={tip}
        onClick={() => void copy(text())}
        size={28}
      />
      {failure !== null && (
        <span className={styles.failure} role="alert">
          {failure}
        </span>
      )}
    </>
  );
}
