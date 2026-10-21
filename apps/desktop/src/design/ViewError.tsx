import { es } from "../i18n/es";
import styles from "./ViewError.module.css";

export interface ViewErrorProps {
  error: string | null;
}

/**
 * The store's last failure, shown inside the view it happened in. Every view
 * renders one: a command that fails must never look like nothing happened.
 */
export function ViewError({ error }: ViewErrorProps) {
  if (!error) return null;
  return (
    <div aria-label={es.errors.title} className={styles.viewError} role="alert">
      {error}
    </div>
  );
}
