import { createRoot } from "react-dom/client";
import { App } from "../app/App";
import { PanelApp } from "../features/call-panel/PanelApp";
import { PanelPort } from "../bridge/panel-port";
import { TauriPort } from "../bridge/tauri-port";
import { baseHub, baseViews } from "./base";
import { scenarios } from "./catalog";
import { startShowcaseClock } from "./clock";
import { NOW } from "./data/time";
import { ShowcaseIndex } from "./ShowcaseIndex";
import { installDaemon, publish } from "./ipc";
import type { Scenario } from "./scenario";
import "../design/fonts.css";
import "../design/tokens.css";
import "../styles.css";

/** What the capture script reads: the scenarios, and when the one shown is settled. */
declare global {
  interface Window {
    __showcase?: { scenarios: Scenario[]; settled: boolean; now: number };
  }
}

const THEME_KEY = "orbit.ui.theme.local";
const ONBOARDING_KEY = "orbit.onboarding.done.local";

/** The browser storage the app starts from: only what the scenario says. */
function prepareStorage(scenario: Scenario, theme: string | null): void {
  window.localStorage.clear();
  if (scenario.onboarded !== false) window.localStorage.setItem(ONBOARDING_KEY, "true");
  if (theme === "light" || theme === "dark") window.localStorage.setItem(THEME_KEY, theme);
  for (const [key, value] of Object.entries(scenario.storage ?? {}))
    window.localStorage.setItem(key, value);
}

function show(scenario: Scenario, root: HTMLElement): void {
  const params = new URLSearchParams(window.location.search);
  prepareStorage(scenario, params.get("tema"));
  if (window.location.hash !== scenario.route) window.location.hash = scenario.route;
  const views = (scenario.views ?? ((base) => base))(baseViews());
  const hub = (scenario.hub ?? ((base) => base))(baseHub());
  // What the daemon says once the window watches, right after its first frames.
  const daemon = installDaemon(scenario, views, hub, () => {
    const later = scenario.later?.(daemon.views);
    void (later ? publish(daemon, later) : Promise.resolve()).then(() => {
      if (window.__showcase) window.__showcase.settled = true;
    });
  });
  const port = scenario.panel === true ? new PanelPort() : new TauriPort();
  // No StrictMode: the showcase connects once, as the built app does.
  createRoot(root).render(scenario.panel === true ? <PanelApp port={port} /> : <App port={port} />);
}

startShowcaseClock();
window.__showcase = { scenarios, settled: false, now: NOW };
const root = document.getElementById("root");
const id = new URLSearchParams(window.location.search).get("s");
const scenario = scenarios.find((candidate) => candidate.id === id);
if (root && scenario) show(scenario, root);
else if (root) createRoot(root).render(<ShowcaseIndex scenarios={scenarios} />);
