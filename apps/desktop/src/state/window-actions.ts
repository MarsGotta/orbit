import { create } from "zustand";

/** Existing contextual handlers. Only the last deleted template is retained, in memory. */
export const useWindowActions = create<{
  findTranscript: (() => void) | null;
  undoTemplate: (() => void) | null;
}>(() => ({ findTranscript: null, undoTemplate: null }));
