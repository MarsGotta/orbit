import { useStore } from "zustand";
import type { OrbitStore } from "../state/orbit-store";
import type { ViewId } from "../state/ui";
import { messageFor } from "./visible-error";

/** The refusals shown inside the view, onboarding and launcher that sent them. */
export function useErrorSelection(store: OrbitStore, view: ViewId) {
  const messages = useStore(store, (state) => state.messages);
  const errorView = window.location.hash.startsWith("#/notes/") ? "notes" : view;
  return {
    errorView,
    visibleError: messageFor(errorView, messages)?.text ?? null,
    onboardingError: messageFor("onboarding", messages)?.text ?? null,
    recordingError: messageFor("recording", messages)?.text ?? null,
  };
}
