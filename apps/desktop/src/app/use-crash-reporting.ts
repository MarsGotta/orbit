import { useEffect } from "react";
import type { OrbitState } from "../state/orbit-store";

/**
 * An error nobody caught, in a handler or a promise, goes through the
 * interceptor like any other failure (SIM-21): it is reported and shown as
 * the generic message, never lost in the console.
 */
export function useCrashReporting(crash: OrbitState["crash"]): void {
  useEffect(() => {
    const uncaught = () => void crash("ui_uncaught_error");
    window.addEventListener("error", uncaught);
    window.addEventListener("unhandledrejection", uncaught);
    return () => {
      window.removeEventListener("error", uncaught);
      window.removeEventListener("unhandledrejection", uncaught);
    };
  }, [crash]);
}
