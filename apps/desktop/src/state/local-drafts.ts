import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Drafts the person wrote and has not sent yet, kept in this Mac's webview
 * so that a view that unmounts, a quit or a fall does not take them. A
 * per-viewer convenience: they never reach the daemon or the hub, and a draft
 * is let go of once what it held was sent or dropped on purpose.
 *
 * Every key starts with this prefix (`test-setup.ts` clears them between tests).
 */
export const DRAFT_PREFIX = "orbit.draft.";

// This page's copy, for a webview that cannot keep local data: a view that
// unmounts still finds its draft while Orbit stays open.
const memory = new Map<string, string>();
// Keys this page already read or wrote: what storage holds for one of them is
// this page's own, not something recovered from an earlier run.
const known = new Set<string>();

function stored(key: string): string | null {
  try {
    return window.localStorage.getItem(DRAFT_PREFIX + key);
  } catch {
    return memory.get(key) ?? null;
  }
}

function keep(key: string, raw: string | null): void {
  if (raw === null) memory.delete(key);
  else memory.set(key, raw);
  try {
    if (raw === null) window.localStorage.removeItem(DRAFT_PREFIX + key);
    else window.localStorage.setItem(DRAFT_PREFIX + key, raw);
  } catch {
    // Full or unavailable: the draft stays on screen and in this page's copy.
  }
}

interface Read<T> {
  value: T;
  /** Left by an earlier run of Orbit (closed or fallen), not by this one. */
  recovered: boolean;
}

function read<T>(key: string | null, empty: T, accept: (raw: unknown) => T | null): Read<T> {
  if (key === null) return { value: empty, recovered: false };
  const raw = stored(key);
  if (raw === null) return { value: empty, recovered: false };
  let parsed: T | null = null;
  try {
    parsed = accept(JSON.parse(raw));
  } catch {
    parsed = null;
  }
  // One this version cannot read is not shown half-understood; the next
  // change replaces it.
  if (parsed === null) return { value: empty, recovered: false };
  return { value: parsed, recovered: !known.has(key) };
}

/** A text draft: any string. */
export function asText(raw: unknown): string | null {
  return typeof raw === "string" ? raw : null;
}

/**
 * The draft under `key` (none with `null`): kept on every change, so nothing
 * needs flushing before a quit, and `empty` lets it go. `recovered` says it
 * came back from an earlier run, until the person changes it.
 */
export function useLocalDraft<T>(
  key: string | null,
  empty: T,
  accept: (raw: unknown) => T | null,
): { value: T; set: (next: T | ((current: T) => T)) => void; recovered: boolean } {
  const [state, setState] = useState(() => ({ key, ...read(key, empty, accept) }));
  const current = state.key === key ? state : { key, ...read(key, empty, accept) };
  if (current !== state) setState(current);
  const latest = useRef(current);
  latest.current = current;
  const set = useCallback(
    (next: T | ((value: T) => T)) => {
      const now = latest.current;
      const value = typeof next === "function" ? (next as (value: T) => T)(now.value) : next;
      if (now.key !== null) {
        known.add(now.key);
        keep(now.key, Object.is(value, empty) ? null : JSON.stringify(value));
      }
      latest.current = { key: now.key, value, recovered: false };
      setState(latest.current);
    },
    [empty],
  );
  useEffect(() => {
    if (key !== null) known.add(key);
  }, [key]);
  return { value: current.value, set, recovered: current.recovered };
}

/**
 * A draft of a text someone else also holds (the hub's personality, a
 * skill): what was written, and the text it was written over. It comes back
 * only over that same text; once the hub holds another, it is not the draft
 * of what is on screen.
 */
export interface TextOver {
  from: string;
  text: string;
}

export function asTextOver(raw: unknown): TextOver | null {
  if (typeof raw !== "object" || raw === null) return null;
  const { from, text } = raw as Record<string, unknown>;
  return typeof from === "string" && typeof text === "string" ? { from, text } : null;
}
