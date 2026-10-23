import { useEffect, useRef } from "react";
import styles from "./Brand.module.css";
import { useReducedMotion } from "./use-reduced-motion";

// The brand's small symbol, from the brand pack's files (1-logo/svg), path
// for path: never redrawn. Its colours are the theme's tokens, so one drawing
// serves both themes as the pack's «color-oscuro» and «color-claro» files do:
// the ring and «tú» in the text's colour, Deimos in --o-accText.

/** The small symbol's ring, with the gap Deimos cuts in it. */
const SMALL_RING =
  "M42.8 24C42.8 34.383 34.383 42.8 24 42.8C13.617 42.8 5.2 34.383 5.2 24C5.2 13.617 13.617 5.2 24 5.2C26.918 5.2 29.68 5.865 32.144 7.051C31.181 7.832 30.409 8.839 29.909 9.991C28.093 9.224 26.096 8.8 24 8.8C15.605 8.8 8.8 15.605 8.8 24C8.8 32.395 15.605 39.2 24 39.2C32.395 39.2 39.2 32.395 39.2 24C39.2 22.836 39.069 21.702 38.821 20.613C40.057 20.319 41.179 19.729 42.106 18.923C42.558 20.538 42.8 22.241 42.8 24Z";
const SMALL_CENTRE =
  "M30.5 24C30.5 27.59 27.59 30.5 24 30.5C20.41 30.5 17.5 27.59 17.5 24C17.5 20.41 20.41 17.5 24 17.5C27.59 17.5 30.5 20.41 30.5 24Z";
const SMALL_DEIMOS =
  "M41.273 13.073C41.273 15.42 39.37 17.323 37.023 17.323C34.676 17.323 32.773 15.42 32.773 13.073C32.773 10.725 34.676 8.823 37.023 8.823C39.37 8.823 41.273 10.725 41.273 13.073Z";

/** One turn of Deimos, as the brand manual's motion page has it. */
const TURN_MS = 12_000;
/** Back to its place when the work ends: under a second, then it is still. */
const RETURN_MS = 600;

/**
 * Turns the ring and Deimos while `turning`; when it ends, eases them back
 * to their place the short way round. With «Reducir movimiento» nothing
 * moves: no turn, and no way back. Web Animations rather than a CSS
 * animation, because only it says how far the turn had gone.
 */
function useOrbit(turning: boolean, still: boolean) {
  const orbit = useRef<SVGGElement>(null);
  const stillNow = useRef(still);
  stillNow.current = still;
  useEffect(() => {
    const group = orbit.current;
    if (!turning || !group || typeof group.animate !== "function") return;
    const turn = group.animate([{ transform: "rotate(0deg)" }, { transform: "rotate(360deg)" }], {
      duration: TURN_MS,
      iterations: Infinity,
    });
    return () => {
      const angle = ((Number(turn.currentTime ?? 0) % TURN_MS) / TURN_MS) * 360;
      turn.cancel();
      if (stillNow.current || angle === 0) return;
      group.animate(
        [
          { transform: `rotate(${angle}deg)` },
          { transform: `rotate(${angle > 180 ? 360 : 0}deg)` },
        ],
        { duration: RETURN_MS, easing: "ease-out" },
      );
    };
  }, [turning]);
  return orbit;
}

export interface BrandMarkProps {
  /**
   * Deimos goes round its orbit, clockwise, once every 12 s: an assistant is
   * working. The ring turns with it, so the gap it cuts stays beside it; the
   * pack has no drawing of the ring without it.
   */
  orbiting?: boolean;
}

/**
 * The small symbol, up to 20 px: one orbit, «tú» and Deimos. Beside «Orbit»
 * it says nothing of its own.
 */
export function BrandMark({ orbiting = false }: BrandMarkProps) {
  const still = useReducedMotion();
  const turning = orbiting && !still;
  const orbit = useOrbit(turning, still);
  return (
    <svg
      aria-hidden="true"
      className={styles.mark}
      data-orbiting={turning}
      height={20}
      viewBox="0 0 48 48"
      width={20}
    >
      <g className={styles.orbit} ref={orbit}>
        <path d={SMALL_RING} fill="currentColor" />
        <path d={SMALL_DEIMOS} fill="var(--o-accText)" />
      </g>
      <path d={SMALL_CENTRE} fill="currentColor" />
    </svg>
  );
}
