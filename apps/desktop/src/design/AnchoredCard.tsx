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
  /** Above its control whenever it fits there, not only when below does not. */
  above?: boolean;
  /** The id of its title. */
  labelledBy: string;
  onClose: () => void;
  onKeyDown?: (event: KeyboardEvent<HTMLElement>) => void;
  children: ReactNode;
}

type Placement = { side: "below" | "above"; style: CSSProperties };

/**
 * Below its control when it fits, above it when not; within the window either
 * way. Where it fits on neither side it takes the side with more room and
 * scrolls inside, so its buttons are always in reach.
 */
function place(
  anchor: HTMLElement,
  card: HTMLElement,
  align: "start" | "end",
  above: boolean,
): Placement {
  const at = anchor.getBoundingClientRect();
  const size = card.getBoundingClientRect();
  // What it holds, even while it scrolls inside a smaller box.
  const height = Math.max(size.height, card.scrollHeight);
  const roomBelow = window.innerHeight - MARGIN - (at.bottom + GAP);
  const roomAbove = at.top - GAP - MARGIN;
  const fitsBelow = height <= roomBelow;
  const fitsAbove = height <= roomAbove;
  const wanted = align === "end" ? at.right - size.width : at.left;
  const left = Math.max(MARGIN, Math.min(wanted, window.innerWidth - size.width - MARGIN));
  const below = above
    ? !fitsAbove && (fitsBelow || roomBelow >= roomAbove)
    : fitsBelow || (!fitsAbove && roomBelow >= roomAbove);
  if (below) return { side: "below", style: { top: at.bottom + GAP, left, maxHeight: roomBelow } };
  // Held by its bottom edge, so a list that opens inside it grows upwards.
  return {
    side: "above",
    style: { bottom: window.innerHeight - at.top + GAP, left, maxHeight: roomAbove },
  };
}

/**
 * The card the edit menu and the anchored confirmation share: a window beside
 * its control, over everything else. Its focus starts on its first field (or
 * button), stays inside it, and goes back to the control when it closes. Esc
 * or a press outside it closes it.
 */
export function AnchoredCard(props: AnchoredCardProps) {
  const { anchor, width, align = "start", above = false, labelledBy, onClose, onKeyDown } = props;
  const { children } = props;
  const card = useModalFocus(true, anchor);
  const [placement, setPlacement] = useState<Placement>({ side: "below", style: {} });
  useEffect(() => {
    card.current?.querySelector<HTMLElement>(FIELDS)?.focus();
  }, [card]);
  useLayoutEffect(() => {
    const update = () => {
      if (!anchor || !card.current) return;
      const next = place(anchor, card.current, align, above);
      setPlacement((now) => (JSON.stringify(now) === JSON.stringify(next) ? now : next));
    };
    update();
    // A list that opens inside it makes it taller: it is placed again.
    const grows = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    if (card.current) grows?.observe(card.current);
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      grows?.disconnect();
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [anchor, align, above, card]);
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
