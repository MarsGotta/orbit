import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { PanelApp } from "./features/call-panel/PanelApp";
import { isPanelWindow } from "./features/call-panel/window";
import type { DaemonPort } from "orbit-ui/bridge/port";
import { installAttachments } from "orbit-ui/bridge/attachments";
import { installClipboard } from "orbit-ui/bridge/clipboard";
import * as attachments from "./bridge/attachments";
import * as clipboard from "./bridge/clipboard";
import { PanelPort } from "./bridge/panel-port";
import { TauriPort } from "./bridge/tauri-port";
import { followTheme } from "orbit-ui/design/theme";
import "orbit-ui/design/fonts.css";
import "orbit-ui/design/tokens.css";
import "./styles.css";

function choosePort(): DaemonPort {
  return isPanelWindow() ? new PanelPort() : new TauriPort();
}

installClipboard(clipboard);
installAttachments(attachments);
// Both windows, the main one and the floating panel, show the same theme.
followTheme();
const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      {isPanelWindow() ? <PanelApp port={choosePort()} /> : <App port={choosePort()} />}
    </StrictMode>,
  );
}
