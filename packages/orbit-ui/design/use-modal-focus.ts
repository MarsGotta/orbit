import { useEffect, useRef } from "react";

/**
 * Keep keyboard focus in an open modal and return it to its caller on close:
 * to `returnTo` when given (the control a menu hangs from, which WebKit does
 * not focus on a press), otherwise to whatever had the focus before.
 */
export function useModalFocus(open: boolean, returnTo?: HTMLElement | null) {
  const panel = useRef<HTMLElement>(null);
  const back = useRef(returnTo);
  back.current = returnTo;
  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement;
    const element = panel.current;
    element?.querySelector<HTMLElement>("input, button")?.focus();
    const trap = (event: KeyboardEvent) => {
      if (event.key !== "Tab" || !element) return;
      const controls = Array.from(
        element.querySelectorAll<HTMLElement>("input:not(:disabled), button:not(:disabled)"),
      ).filter(
        (control) =>
          !(control instanceof HTMLInputElement && control.type === "radio" && !control.checked),
      );
      const first = controls[0];
      const last = controls[controls.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    };
    element?.addEventListener("keydown", trap);
    return () => {
      element?.removeEventListener("keydown", trap);
      const target = back.current?.isConnected ? back.current : previous;
      if (target instanceof HTMLElement && target.isConnected) target.focus();
    };
  }, [open]);
  return panel;
}
