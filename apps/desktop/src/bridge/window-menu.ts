export type MenuAction =
  | "settings"
  | "record"
  | "finish"
  | "new_conversation"
  | "find"
  | "stop"
  | "sidebar"
  | "bigger"
  | "smaller"
  | "actual_size"
  | "recover_template";

/** The main window's current handlers and local presentation state, never content. */
export interface MenuState {
  record: boolean;
  finish: boolean;
  new_conversation: boolean;
  find: boolean;
  stop: boolean;
  sidebar: boolean;
  bigger: boolean;
  smaller: boolean;
  actual_size: boolean;
  recover_template: boolean;
}
