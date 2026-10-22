/**
 * The registry of what is in flight, for the components that draw loading:
 * the general loader, a busy button, a skeleton. The window's store is given
 * once, at its root; without it (a component rendered alone) nothing is in
 * flight and every call runs untracked.
 *
 * - `useInFlight(filter)`: whether anything matching is in flight: by feature,
 *   by order or call (`command`), by the view it was sent from (`origin`),
 *   or, with no filter, anything at all;
 * - `useNoStateYet(feature)`: the feature has no state yet (draw its skeleton,
 *   never a «no»);
 * - `useTrack()`: runs a call to the app outside `send` through the registry.
 */
import { createContext, useContext, useSyncExternalStore, type ReactNode } from "react";
import type { FeatureId } from "../bridge/generated";
import { hasNoStateYet, inFlight, type WaitFilter } from "./in-flight";
import type { OrbitState, OrbitStore } from "./orbit-store";

const StoreContext = createContext<OrbitStore | null>(null);

export function OrbitStoreProvider({
  store,
  children,
}: {
  store: OrbitStore;
  children: ReactNode;
}) {
  return <StoreContext.Provider value={store}>{children}</StoreContext.Provider>;
}

const unsubscribed = () => () => undefined;

function useSelect<T>(select: (state: OrbitState) => T, fallback: T): T {
  const store = useContext(StoreContext);
  return useSyncExternalStore(store?.subscribe ?? unsubscribed, () =>
    store ? select(store.getState()) : fallback,
  );
}

export function useInFlight(filter: WaitFilter = {}): boolean {
  const { feature, command, origin } = filter;
  return useSelect(
    (state) => inFlight(state.waits, { feature, command, origin }).length > 0,
    false,
  );
}

export function useNoStateYet(feature: FeatureId): boolean {
  return useSelect((state) => hasNoStateYet(state.features, feature), false);
}

const untracked: OrbitState["track"] = (_call, _feature, work) => work();

export function useTrack(): OrbitState["track"] {
  return useContext(StoreContext)?.getState().track ?? untracked;
}
