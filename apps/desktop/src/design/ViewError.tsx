import { useCopy } from "../i18n/CopyProvider";
import { createContext, useContext, type ReactNode } from "react";

import styles from "./ViewError.module.css";

/**
 * What the window offers beside a view's failure («Reintentar», «Copiar el
 * código»), given its words. The window knows the message behind them; a
 * view only passes the words it was given.
 */
export const ViewErrorActions = createContext<((error: string) => ReactNode) | null>(null);

export interface ViewErrorProps {
  error: string | null;
}

/**
 * The store's last failure, shown inside the view it happened in. Every view
 * renders one: a command that fails must never look like nothing happened.
 */
export function ViewError({ error }: ViewErrorProps) {
  const es = useCopy();
  const actions = useContext(ViewErrorActions);
  if (!error) return null;
  return (
    <div aria-label={es.errors.title} className={styles.viewError} role="alert">
      <span className={styles.text}>{error}</span>
      {actions?.(error)}
    </div>
  );
}
