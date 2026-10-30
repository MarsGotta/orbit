import { useEffect, useRef } from "react";
import type { DaemonPort } from "../bridge/port";
import type { AppCommand } from "../bridge/generated";
import type { MenuAction, MenuState } from "../bridge/window-menu";
import type { AppView } from "../state/app-view";
import { useUiStore } from "../state/ui";
import { followWindowPreferences, useWindowPreferences } from "../state/window-preferences";
import { useWindowActions } from "../state/window-actions";
import { goTo } from "../features/assistant/assistant-route";

function menuState(snapshot: AppView | null, find: boolean, recoverTemplate: boolean): MenuState {
  const { sidebar, scale } = useWindowPreferences.getState();
  const ready = snapshot?.connection.daemon === "ready";
  const current = ready && snapshot?.recording_availability?.state === "current";
  return {
    sidebar,
    bigger: scale < 2,
    smaller: scale > 1,
    actual_size: scale !== 1,
    find,
    recover_template: recoverTemplate,
    record: current && snapshot.meeting.phase === "idle",
    finish: current && ["starting", "recording", "paused"].includes(snapshot.meeting.phase),
    new_conversation: ready && snapshot?.chat.availability?.state === "current",
    stop: ready,
  };
}

function run(
  action: MenuAction,
  state: MenuState,
  send: (command: AppCommand) => Promise<unknown>,
  record: () => void,
) {
  const actions = useWindowActions.getState();
  const preferences = useWindowPreferences.getState();
  switch (action) {
    case "settings":
      useUiStore.getState().openSettings("general");
      break;
    case "new_conversation":
      if (state.new_conversation) {
        useUiStore.getState().navigate("assistant");
        goTo({ screen: "new" });
      }
      break;
    case "record":
      if (state.record) record();
      break;
    case "finish":
      if (state.finish) void send({ command: "meeting_finish" });
      break;
    case "stop":
      if (state.stop) void send({ command: "privacy_stop" });
      break;
    case "find":
      actions.findTranscript?.();
      break;
    case "recover_template":
      actions.undoTemplate?.();
      break;
    case "sidebar":
      preferences.toggleSidebar();
      break;
    case "bigger":
      preferences.resizeText(0.1);
      break;
    case "smaller":
      preferences.resizeText(-0.1);
      break;
    case "actual_size":
      preferences.resizeText("reset");
      break;
  }
}

/** Native menu selections use the same handlers and availability as the main window. */
export function useWindowMenu(
  port: DaemonPort,
  snapshot: AppView | null,
  send: (command: AppCommand) => Promise<unknown>,
  record: () => void,
) {
  const { findTranscript, undoTemplate } = useWindowActions();
  const { sidebar, scale } = useWindowPreferences();
  const state = menuState(snapshot, findTranscript !== null, undoTemplate !== null);
  const latest = useRef((action: MenuAction) => run(action, state, send, record));
  latest.current = (action) => run(action, state, send, record);
  useEffect(() => followWindowPreferences(), []);
  useEffect(() => {
    let active = true;
    let stop = () => {};
    void port
      .onMenuAction?.((action) => latest.current(action))
      .then((unsubscribe) => {
        if (active) stop = unsubscribe;
        else unsubscribe();
      });
    return () => {
      active = false;
      stop();
    };
  }, [port]);
  const serialized = JSON.stringify(state);
  useEffect(() => {
    void port.setMenuState?.(JSON.parse(serialized) as MenuState);
  }, [port, serialized, sidebar, scale]);
}
