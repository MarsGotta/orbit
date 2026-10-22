import { useCallback, useState } from "react";
import { useStore } from "zustand";
import type { DaemonPort } from "../bridge/port";
import { es } from "../i18n/es";
import { createOrbitStore } from "../state/orbit-store";
import { useCrashReporting } from "./use-crash-reporting";
import { useOrbitConnection } from "./useOrbitConnection";

/** The main window's store, connected, with uncaught errors routed to the interceptor. */
export function useAppStore(port: DaemonPort) {
  const [store] = useState(() => createOrbitStore(port));
  const storeCrash = useStore(store, (state) => state.crash);
  useOrbitConnection(useStore(store, (state) => state.connect));
  useCrashReporting(storeCrash);
  return {
    store,
    snapshot: useStore(store, (state) => state.snapshot),
    // A failed connection says what to do; its cause is not for the screen.
    error: useStore(store, (state) => (state.error === null ? null : es.connectFailed)),
    send: useStore(store, (state) => state.send),
    messages: useStore(store, (state) => state.messages),
    dismiss: useStore(store, (state) => state.dismiss),
    retry: useStore(store, (state) => state.retry),
    crash: useCallback(() => storeCrash("ui_render_failed"), [storeCrash]),
  };
}
