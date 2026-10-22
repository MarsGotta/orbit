import { useEffect, useId, type ReactNode } from "react";
import { es } from "../i18n/es";
import { Icon } from "./Icon";
import { useModalFocus } from "./use-modal-focus";
import styles from "./Dialog.module.css";

export interface DialogProps {
  title: string;
  subtitle?: string;
  children: ReactNode;
  /** The actions at the bottom right: a quiet «Cancelar» and the one main action. */
  footer?: ReactNode;
  onClose: () => void;
}

const FIELDS = "input:not(:disabled), textarea:not(:disabled), select:not(:disabled)";

/**
 * A window over the view. Esc, a press outside it or its × close it; the focus
 * goes to its first field (or first button) and back to whatever opened it.
 */
export function Dialog({ title, subtitle, children, footer, onClose }: DialogProps) {
  const titleId = useId();
  const subtitleId = useId();
  const panel = useModalFocus(true);
  useEffect(() => {
    panel.current?.querySelector<HTMLElement>(FIELDS)?.focus();
  }, [panel]);
  return (
    <div
      className={styles.scrim}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <section
        aria-describedby={subtitle ? subtitleId : undefined}
        aria-labelledby={titleId}
        aria-modal="true"
        className={styles.dialog}
        onKeyDown={(event) => {
          if (event.key !== "Escape") return;
          event.stopPropagation();
          onClose();
        }}
        ref={panel}
        role="dialog"
      >
        <header className={styles.header}>
          <div>
            <h2 className={styles.title} id={titleId}>
              {title}
            </h2>
            {subtitle && (
              <p className={styles.subtitle} id={subtitleId}>
                {subtitle}
              </p>
            )}
          </div>
          <button
            aria-label={es.controls.close}
            className={styles.close}
            onClick={onClose}
            type="button"
          >
            <Icon name="close" size={16} />
          </button>
        </header>
        <div className={styles.content}>{children}</div>
        {footer && (
          <footer className={styles.footer}>
            <span className={styles.hint}>{es.controls.escToClose}</span>
            <div className={styles.actions}>{footer}</div>
          </footer>
        )}
      </section>
    </div>
  );
}
