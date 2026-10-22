import type { FeatureFrame, FeatureId } from "../bridge/generated";
import type { AgentConfig } from "../bridge/generated/AgentConfig";
import type { AppCommand } from "../bridge/generated/AppCommand";
import type { ChatMessage } from "../bridge/generated/ChatMessage";
import type { ChatSessionSummary } from "../bridge/generated/ChatSessionSummary";
import type { SkillDocument } from "../bridge/generated/SkillDocument";
import type { Refusal } from "../bridge/refusal";

/** What the daemon publishes for each function, as the generated types say. */
export type Views = { [F in FeatureId]: Extract<FeatureFrame, { feature: F }>["view"] };

/** What the hub answers the reads a view makes itself. */
export interface HubData {
  sessions: ChatSessionSummary[];
  messages: Record<string, ChatMessage[]>;
  agents: Record<string, AgentConfig>;
  skills: Record<string, SkillDocument>;
}

/**
 * One step the capture takes before its picture, as Playwright takes it:
 * `selector` is a Playwright selector (`role=button[name="Enviar"]`, `text=…`).
 */
export type Step =
  | { click: string }
  | { hover: string }
  | { press: string }
  | { fill: string; text: string }
  /** Scrolls the box that holds it so it sits at the top, or with `end` at the bottom. */
  | { scroll: string; align?: "end" }
  | { wait: string }
  /** A press of the mouse `at` points from its top left, where no control is to aim at. */
  | { mouse: string; at: [number, number] }
  /** Drags it by `by` points with the mouse. */
  | { drag: string; by: [number, number] }
  /** Moves the page's clock this many milliseconds on, for a time that counts. */
  | { advance: number };

export interface Scenario {
  /** The file name and the `?s=` of the page. */
  id: string;
  /** The view, in Spanish, as the redesign brief names it. */
  view: string;
  /** The state it shows, in Spanish. */
  state: string;
  /** One line, in Spanish, of what the picture shows. */
  description: string;
  /** The floating call panel instead of the main window. */
  panel?: boolean;
  /** The design system's sheet instead of the app: its components in their states, no daemon. */
  sheet?: boolean;
  /**
   * The window's size in points, as the app gives it: the main window is
   * 1440 × 900 and the panel 360 × 120, or 360 × 300 with a question
   * (`tauri.conf.json`, `src-tauri/src/panel.rs`).
   */
  size?: { width: number; height: number };
  /** The route the window opens on, `#/assistant` or `#/notes/<id>`. */
  route: string;
  /** What the daemon publishes at startup, from the showcase's ordinary day. */
  views?: (base: Views) => Views;
  /** What the daemon publishes once the window watches: notices, a new turn. */
  later?: (views: Views) => Partial<Views>;
  hub?: (base: HubData) => HubData;
  /** The daemon never answers the first snapshot, or refuses it: the app before its first frames. */
  connect?: "hang" | "fail";
  /** The refusal the daemon answers an order with, when it refuses it. */
  refuse?: (command: AppCommand) => Refusal | null;
  /** The browser storage the app reads at start, beyond onboarding done. */
  storage?: Record<string, string>;
  /** Whether the first-run guide was completed on this Mac (true by default). */
  onboarded?: boolean;
  /** The selector of what must be on screen before the picture. */
  ready: string;
  steps?: Step[];
}
