import { useCopy } from "../i18n/CopyProvider";
import { useCallback, type ReactNode } from "react";
import { copyText } from "../bridge/clipboard";
import { Button } from "../design/Button";
import { useFeedback } from "../design/feedback";
import { ViewErrorActions } from "../design/ViewError";

import { localizeMessage, type Failure, type Message } from "./interceptor";
import styles from "./FailureActions.module.css";

/** «Copiar el código» → «✓ Copiado»: only the code goes to the clipboard. */
function CopyCode({ code }: { code: string }) {
  const es = useCopy();
  const { feedback, track } = useFeedback();
  return (
    <Button
      className={styles.action}
      doneLabel={es.failure.copied}
      feedback={feedback}
      onClick={() => track(copyText(code))}
      pendingLabel={es.failure.copying}
      size={28}
      variant="outline"
    >
      {es.failure.copyCode}
    </Button>
  );
}

/** A failure's two ways out: «Reintentar» (the main one) when its order can be repeated. */
export function FailureActions({
  failure,
  onRetry,
  op,
}: {
  failure: Failure;
  onRetry: (op: string) => void;
  op: string;
}) {
  const es = useCopy();
  return (
    <span className={styles.actions}>
      {failure.retry && (
        <Button className={styles.action} onClick={() => onRetry(op)} size={28} variant="accent">
          {es.failure.retry}
        </Button>
      )}
      <CopyCode code={failure.code} />
    </span>
  );
}

/** What to do and the code, set apart in the help of a generic failure. */
export function FailureHelp({ code }: { code: string }) {
  const es = useCopy();
  return (
    <>
      {es.failure.helpBefore}
      <span className={styles.code}>{code}</span>
      {es.failure.helpAfter}
    </>
  );
}

/** Gives every view's failure the actions of the message behind its words. */
export function FailureActionsProvider({
  children,
  messages,
  onRetry,
}: {
  children: ReactNode;
  messages: readonly Message[];
  onRetry: (op: string) => void;
}) {
  const copy = useCopy();
  const actions = useCallback(
    (error: string) => {
      const message = messages
        .map(localizeMessage)
        .find((candidate) => candidate.failure !== undefined && candidate.text === error);
      if (!message?.failure) return null;
      return <FailureActions failure={message.failure} onRetry={onRetry} op={message.op} />;
    },
    [messages, onRetry, copy],
  );
  return <ViewErrorActions value={actions}>{children}</ViewErrorActions>;
}
