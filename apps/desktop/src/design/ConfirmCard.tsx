import { useId, type ReactNode } from "react";
import { es } from "../i18n/es";
import { AnchoredCard } from "./AnchoredCard";
import { Button } from "./Button";
import cardStyles from "./AnchoredCard.module.css";
import styles from "./ConfirmCard.module.css";

// Within the design's 340 to 380, as wide as the prototype draws both «Rehacer».
const WIDTH = 350;

export interface ConfirmCardProps {
  /** The button that asked: the card hangs from it and gives it the focus back. */
  anchor: HTMLElement | null;
  align?: "start" | "end";
  /** The question: «¿Volver a transcribir la reunión?». */
  title: string;
  /** What it will do, how long it takes and what stays. */
  children: ReactNode;
  /** A smaller line under it: what does not change. */
  note?: string;
  /** The same verb as the button that opened it: «Rehacer transcripción». */
  confirmLabel: string;
  onConfirm: () => void;
  onClose: () => void;
}

/**
 * The anchored confirmation, for what takes a while or changes something that
 * exists. What can be undone is not confirmed: it offers «Deshacer» instead.
 */
export function ConfirmCard(props: ConfirmCardProps) {
  const { anchor, align = "end", title, children, note, confirmLabel, onConfirm, onClose } = props;
  const titleId = useId();
  return (
    <AnchoredCard
      align={align}
      anchor={anchor}
      labelledBy={titleId}
      onClose={onClose}
      width={WIDTH}
    >
      <h3 className={cardStyles.title} id={titleId}>
        {title}
      </h3>
      <p className={styles.text}>{children}</p>
      {note && <p className={styles.note}>{note}</p>}
      <footer className={cardStyles.footer}>
        <Button onClick={onClose} size={32} variant="ghost">
          {es.controls.cancel}
        </Button>
        <Button onClick={onConfirm} size={32} variant="accent">
          {confirmLabel}
        </Button>
      </footer>
    </AnchoredCard>
  );
}
