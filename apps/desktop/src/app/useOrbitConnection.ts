import { useEffect } from "react";
import type { OrbitState } from "../state/orbit-store";
import type { Unsubscribe } from "../bridge/port";

export function useOrbitConnection(connect: OrbitState["connect"]): void {
  useEffect(() => {
    let active = true;
    let unsubscribe: Unsubscribe = () => undefined;
    void connect().then((stop) => {
      if (active) unsubscribe = stop;
      else stop();
    });
    return () => {
      active = false;
      unsubscribe();
    };
  }, [connect]);
}
