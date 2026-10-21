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
import styles from "./Toast.module.css";

const STACK_ID = "orbit-notices";
// Five seconds is enough to read a short confirmation without covering the view.
const DONE_VISIBLE_MS = 5_000;
// A notice the reader can act on (undo a deletion, open the saved note) stays
// ten seconds: reaching its button takes longer than reading it (Mars, 2026-10-20).
const ACTIONABLE_VISIBLE_MS = 10_000;

export function ToastStack() {
  return <div className={styles.stack} id={STACK_ID} />;
}

export interface ToastProps {
  embedded?: boolean;
  persistent?: boolean;
  message: string;
  kind?: Exclude<NoticeClass, "silent">;
  action?: ReactNode;
  onClose: () => void;
  onUndo?: () => void;
}

/** One presentation and lifecycle for the interceptor and local confirmations. */
export function Toast({ persistent, ...props }: ToastProps) {
  const { embedded, message, kind = "done", action, onClose, onUndo } = props;
  const [count, setCount] = useState(ACTIONABLE_VISIBLE_MS / 1_000);
  const [visible, setVisible] = useState(true);
  const [stack, setStack] = useState<HTMLElement | null>(null);
  const close = useRef(onClose);
  const hasUndo = onUndo !== undefined;
  const duration = hasUndo || action !== undefined ? ACTIONABLE_VISIBLE_MS : DONE_VISIBLE_MS;

  const dismiss = useCallback(() => {
    setVisible(false);
    close.current();
  }, []);

  useEffect(() => {
    close.current = onClose;
  }, [onClose]);
  useEffect(() => {
    if (!embedded) setStack(document.getElementById(STACK_ID));
  }, [embedded]);
  useEffect(() => {
    if (!visible || persistent || kind !== "done") return;
    const timer = window.setTimeout(dismiss, duration);
    return () => window.clearTimeout(timer);
  }, [dismiss, duration, kind, persistent, visible]);
  useEffect(() => {
    if (!visible || !hasUndo) return;
    const interval = window.setInterval(() => {
      setCount((current) => Math.max(0, current - 1));
    }, 1_000);
    return () => window.clearInterval(interval);
  }, [hasUndo, visible]);

  if (!visible) return null;
  const toast = (
    <div
      aria-label={es.shell.notice}
      className={`${styles.toast} ${embedded ? styles.embedded : ""}`}
      data-kind={kind}
      role={kind === "fixable" || kind === "failed" ? "alert" : "status"}
      style={{ "--notice-duration": `${duration}ms` } as CSSProperties}
    >
      <span className={styles.message}>{message}</span>
      {action}
      {onUndo && (
        <button
          onClick={() => {
            onUndo();
            dismiss();
          }}
          type="button"
        >
          {es.design.undo} {count}s
        </button>
      )}
      <button aria-label={es.shell.closeNotice} onClick={dismiss} type="button">
        <span aria-hidden="true">×</span>
      </button>
    </div>
  );
  return stack && !embedded ? createPortal(toast, stack) : toast;
}
