import type { ReactNode } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { AppView } from "../../state/app-view";

type Settings = AppView["settings"];
type Piece = { send: (command: AppCommand) => unknown; settings: Settings };

/**
 * What only a device's own frame can show in Settings, each piece drawn where
 * its section always showed it. Without the `frame` prop none of them appears,
 * nor the two group titles that exist only for them.
 */
export interface SettingsFrameSections {
  /** How many system permissions are missing, for «Falta 1» in the index. */
  missingPermissions: (settings: Settings) => number;
  thisDevice: (props: {
    onReopen: (mode?: "pair") => void;
    settings: Settings;
    statusKnown: boolean;
  }) => ReactNode;
  loginItem: (props: Piece) => ReactNode;
  updates: (props: Piece) => ReactNode;
  systemPermissions: (props: Piece) => ReactNode;
}
