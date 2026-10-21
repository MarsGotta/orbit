import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { PanelApp } from "./features/call-panel/PanelApp";
import { isPanelWindow } from "./features/call-panel/window";
import type { DaemonPort } from "./bridge/port";
import { PanelPort } from "./bridge/panel-port";
import { TauriPort } from "./bridge/tauri-port";
import "./design/fonts.css";
import "./design/tokens.css";
import "./styles.css";

function choosePort(): DaemonPort {
  return isPanelWindow() ? new PanelPort() : new TauriPort();
}

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      {isPanelWindow() ? <PanelApp port={choosePort()} /> : <App port={choosePort()} />}
    </StrictMode>,
  );
}
