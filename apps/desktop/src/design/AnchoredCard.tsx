import {
  useEffect,
  useLayoutEffect,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { useModalFocus } from "./use-modal-focus";
import styles from "./AnchoredCard.module.css";

// 6 points from its control, and at least 8 from the window's edges.
const GAP = 6;
const MARGIN = 8;
const FIELDS = "input:not(:disabled), textarea:not(:disabled), select:not(:disabled)";

export interface AnchoredCardProps {
  /** The control it hangs from; the focus goes back to it on close. */
  anchor: HTMLElement | null;
  width: number;
  /** Which edge of the control it lines up with. */
  align?: "start" | "end";
  /** The id of its title. */
  labelledBy: string;
  onClose: () => void;
  onKeyDown?: (event: KeyboardEvent<HTMLElement>) => void;
  children: ReactNode;
}

type Placement = { side: "below" | "above"; style: CSSProperties };

/** Below its control when it fits, above it when not; within the window either way. */
function place(anchor: HTMLElement, card: HTMLElement, align: "start" | "end"): Placement {
  const at = anchor.getBoundingClientRect();
  const size = card.getBoundingClientRect();
  const fitsBelow = at.bottom + GAP + size.height <= window.innerHeight - MARGIN;
  const fitsAbove = at.top - GAP - size.height >= MARGIN;
  const wanted = align === "end" ? at.right - size.width : at.left;
  const left = Math.max(MARGIN, Math.min(wanted, window.innerWidth - size.width - MARGIN));
  if (fitsBelow || !fitsAbove) return { side: "below", style: { top: at.bottom + GAP, left } };
  // Held by its bottom edge, so a list that opens inside it grows upwards.
  return { side: "above", style: { bottom: window.innerHeight - at.top + GAP, left } };
}

/**
 * The card the edit menu and the anchored confirmation share: a window beside
 * its control, over everything else. Its focus starts on its first field (or
 * button), stays inside it, and goes back to the control when it closes. Esc
 * or a press outside it closes it.
 */
export function AnchoredCard(props: AnchoredCardProps) {
  const { anchor, width, align = "start", labelledBy, onClose, onKeyDown, children } = props;
  const card = useModalFocus(true, anchor);
  const [placement, setPlacement] = useState<Placement>({ side: "below", style: {} });
  useEffect(() => {
    card.current?.querySelector<HTMLElement>(FIELDS)?.focus();
  }, [card]);
  useLayoutEffect(() => {
    const update = () => {
      if (anchor && card.current) setPlacement(place(anchor, card.current, align));
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [anchor, align, card]);
  useEffect(() => {
    const outside = (event: MouseEvent) => {
      if (!(event.target instanceof Node)) return;
      if (card.current?.contains(event.target) || anchor?.contains(event.target)) return;
      onClose();
    };
    document.addEventListener("mousedown", outside);
    return () => document.removeEventListener("mousedown", outside);
  }, [anchor, card, onClose]);
  return createPortal(
    <section
      aria-labelledby={labelledBy}
      className={styles.card}
      data-placement={placement.side}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.stopPropagation();
          onClose();
          return;
        }
        onKeyDown?.(event);
      }}
      ref={card}
      role="dialog"
      style={{ ...placement.style, width }}
    >
      {children}
    </section>,
    document.body,
  );
}
