import { useEffect, useRef } from "react";

/** Keep keyboard focus in an open modal and return it to its caller on close. */
export function useModalFocus(open: boolean) {
  const panel = useRef<HTMLElement>(null);
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
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    };
  }, [open]);
  return panel;
}
