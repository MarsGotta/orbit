import { useEffect, useRef } from "react";
import { calendarKeyAction, type CalendarKeyAction, isTypingTarget } from "./keyboard";

/**
 * The calendar's keys while it is on screen. Nothing happens while
 * the focus is in a text field, or when another handler already used the key.
 */
export function useCalendarKeys(onAction: (action: CalendarKeyAction) => void) {
  const latest = useRef(onAction);
  latest.current = onAction;
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat || isTypingTarget(event.target)) return;
      const action = calendarKeyAction(event);
      if (action === null) return;
      event.preventDefault();
      latest.current(action);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
