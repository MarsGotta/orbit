import {
  createContext,
  useCallback,
  useContext,
  useLayoutEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { useCopy } from "../i18n/CopyProvider";
import styles from "./Shell.module.css";

/**
 * Two places of the window that a view fills from inside itself: the list
 * column beside the sidebar (Reuniones, Asistente) and the start of the top
 * bar, where a view with depth puts its trail or its own header instead of
 * the search box.
 */
interface Slots {
  column: HTMLElement | null;
  leading: HTMLElement | null;
  setColumn: (element: HTMLElement | null) => void;
  setLeading: (element: HTMLElement | null) => void;
  /** How many views fill the top bar's start now: the search box steps aside. */
  leadingClaims: number;
  claimLeading: () => () => void;
}

const SlotsContext = createContext<Slots | null>(null);

export function ShellSlots({ children }: { children: ReactNode }) {
  const [column, setColumn] = useState<HTMLElement | null>(null);
  const [leading, setLeading] = useState<HTMLElement | null>(null);
  const [leadingClaims, setClaims] = useState(0);
  const claimLeading = useCallback(() => {
    setClaims((count) => count + 1);
    return () => setClaims((count) => count - 1);
  }, []);
  const slots = useMemo<Slots>(
    () => ({ column, leading, setColumn, setLeading, leadingClaims, claimLeading }),
    [column, leading, leadingClaims, claimLeading],
  );
  return <SlotsContext value={slots}>{children}</SlotsContext>;
}

export function useShellSlots(): Slots | null {
  return useContext(SlotsContext);
}

/**
 * The 272 px column of a list beside the sidebar, at the window's full
 * height, with its 52 px header (the title and, at its right, its actions).
 */
export function ListColumn({
  label,
  title,
  actions,
  children,
}: {
  /** What the list is, for a screen reader: «Lista de reuniones». */
  label: string;
  title: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  const host = useShellSlots()?.column;
  if (!host) return null;
  return createPortal(
    <aside aria-label={label} className={styles.listColumn}>
      <header className={styles.listHeader}>
        <h2>{title}</h2>
        {actions}
      </header>
      <div className={styles.listBody}>{children}</div>
    </aside>,
    host,
  );
}

/**
 * What a view puts at the start of the top bar instead of the search box.
 * A view drawn outside the window's shell keeps it where the view puts it.
 */
export function TopBarLeading({ children }: { children: ReactNode }) {
  const slots = useShellSlots();
  const claim = slots?.claimLeading;
  useLayoutEffect(() => claim?.(), [claim]);
  if (slots === null) return children;
  if (!slots.leading) return null;
  return createPortal(children, slots.leading);
}

export interface Crumb {
  label: string;
  /** Where it leads; the last one, where you are, has none. */
  href?: string;
}

/** «Ajustes / Privacidad y permisos»: only where a view has depth. */
export function Breadcrumbs({ trail }: { trail: Crumb[] }) {
  const es = useCopy();
  return (
    <nav aria-label={es.shell.breadcrumbsLabel} className={styles.crumbs}>
      <ol>
        {trail.map((crumb, index) => (
          <li key={`${index}-${crumb.label}`}>
            {crumb.href ? (
              <a href={crumb.href}>{crumb.label}</a>
            ) : (
              <span aria-current="page">{crumb.label}</span>
            )}
          </li>
        ))}
      </ol>
    </nav>
  );
}
