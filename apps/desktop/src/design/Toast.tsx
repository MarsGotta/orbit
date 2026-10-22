import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import type { NoticeClass } from "../bridge/generated/NoticeClass";
import { es } from "../i18n/es";
import { Spinner } from "./Spinner";
import { useCornerPlace } from "./toast-queue";
import styles from "./Toast.module.css";

const STACK_ID = "orbit-notices";
// Five seconds is enough to read a short confirmation without covering the view.
const DONE_VISIBLE_MS = 5_000;
// A notice the reader can act on (undo a deletion, open the saved note) stays
// ten seconds: reaching its button takes longer than reading it (reported 2026-10-20).
const ACTIONABLE_VISIBLE_MS = 10_000;

/** The corner, top right, where every notice of the window appears. */
export function ToastStack() {
  return <div className={styles.stack} id={STACK_ID} />;
}

export type ToastKind = Exclude<NoticeClass, "silent">;

export interface ToastProps {
  embedded?: boolean;
  persistent?: boolean;
  /** The title: what happened. */
  message: string;
  /** One line more: why, or what to do. */
  detail?: ReactNode;
  /**
   * `done` goes after 5 s (10 s with an action or «Deshacer»); `working`
   * stays until its work ends; `fixable` and `failed` stay until closed.
   */
  kind?: ToastKind;
  action?: ReactNode;
  onClose: () => void;
  onUndo?: () => void;
}

/** One presentation and lifecycle for the interceptor and local confirmations. */
export function Toast({ persistent, ...props }: ToastProps) {
  const { embedded, kind = "done", action, onClose, onUndo } = props;
  const [visible, setVisible] = useState(true);
  const [stack, setStack] = useState<HTMLElement | null>(null);
  const shown = useCornerPlace(embedded !== true && visible);
  const close = useRef(onClose);
  const duration =
    onUndo !== undefined || action !== undefined ? ACTIONABLE_VISIBLE_MS : DONE_VISIBLE_MS;

  const dismiss = useCallback(() => {
    setVisible(false);
    close.current();
  }, []);

  useEffect(() => {
    close.current = onClose;
  }, [onClose]);
  useEffect(() => {
    if (embedded !== true) setStack(document.getElementById(STACK_ID));
  }, [embedded]);
  useEffect(() => {
    if (!visible || !shown || persistent || kind !== "done") return;
    const timer = window.setTimeout(dismiss, duration);
    return () => window.clearTimeout(timer);
  }, [dismiss, duration, kind, persistent, shown, visible]);

  if (!visible || !shown) return null;
  const toast = (
    <div
      aria-label={es.shell.notice}
      className={`${styles.toast} ${embedded ? styles.embedded : ""}`}
      data-expires={(kind === "done" && !persistent) || undefined}
      data-kind={kind}
      role={kind === "fixable" || kind === "failed" ? "alert" : "status"}
      style={{ "--notice-duration": `${duration}ms` } as CSSProperties}
    >
      <ToastBody {...props} kind={kind} dismiss={dismiss} />
    </div>
  );
  return stack && !embedded ? createPortal(toast, stack) : toast;
}

function ToastBody(props: ToastProps & { kind: ToastKind; dismiss: () => void }) {
  const { message, detail, kind, action, onUndo, dismiss } = props;
  return (
    <>
      <span aria-hidden="true" className={styles.signal} data-signal={kind}>
        {kind === "working" && <Spinner size={12} />}
      </span>
      <div className={styles.body}>
        <span className={styles.message}>{message}</span>
        {detail && <span className={styles.detail}>{detail}</span>}
        {(action !== undefined || onUndo !== undefined) && (
          <div className={styles.actions}>
            {action}
            {onUndo && <UndoButton onUndo={onUndo} dismiss={dismiss} />}
          </div>
        )}
      </div>
      <button
        aria-label={es.shell.closeNotice}
        className={styles.close}
        onClick={dismiss}
        type="button"
      >
        <span aria-hidden="true">×</span>
      </button>
    </>
  );
}

/** «Deshacer 9s»: the real undo, offered while the notice lasts. */
function UndoButton({ onUndo, dismiss }: { onUndo: () => void; dismiss: () => void }) {
  const [count, setCount] = useState(ACTIONABLE_VISIBLE_MS / 1_000);
  useEffect(() => {
    const interval = window.setInterval(() => {
      setCount((current) => Math.max(0, current - 1));
    }, 1_000);
    return () => window.clearInterval(interval);
  }, []);
  return (
    <button
      onClick={() => {
        onUndo();
        dismiss();
      }}
      type="button"
    >
      {es.design.undo} {count}s
    </button>
  );
}
