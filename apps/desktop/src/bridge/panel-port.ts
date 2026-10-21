import { invoke } from "@tauri-apps/api/core";
import { panelOrders } from "./generated/panelOrders";
import type { PanelOrder } from "./generated/PanelOrder";
import type { AppCommand, CommandReceipt } from "./generated";
import type { DaemonPort } from "./port";
import { TauriPort } from "./tauri-port";

/**
 * P9-11: the floating panel's port. It reads and listens like the main
 * window, and sends only its own orders through `orbit_panel_send`, the one
 * its window is allowed.
 */
export class PanelPort extends TauriPort {
  override send(command: AppCommand, op: string): Promise<CommandReceipt> {
    if (!Object.hasOwn(panelOrders, command.command)) {
      return Promise.reject(new Error(`El panel de llamadas no envía la orden ${command.command}`));
    }
    return invoke<CommandReceipt>("orbit_panel_send", { command, op });
  }

  /**
   * P9-12: the panel takes the keyboard only while Mars writes a question
   * to Deimos, and gives it back when she sends or cancels it.
   */
  /** P9-12: a link of the answer, only http(s) (checked again in Rust). */
  openUrl(url: string): Promise<CommandReceipt> {
    return invoke<CommandReceipt>("orbit_panel_send", {
      command: { command: "open_url", url } satisfies PanelOrder,
    });
  }

  focusPanel(focus: boolean): Promise<CommandReceipt> {
    return invoke<CommandReceipt>("orbit_panel_send", {
      command: { command: "panel_focus", focus } satisfies PanelOrder,
    });
  }
}

/**
 * Opens a web link of Deimos's answer through the panel's own order. A
 * refusal (not http/https, credentials, no host) rejects with Rust's cause.
 */
export async function panelOpenUrl(port: DaemonPort, url: string): Promise<void> {
  if (port.openUrl) await port.openUrl(url);
}

/** Asks the panel's window for the keyboard (or gives it back), when it is one. */
export async function panelFocus(port: DaemonPort, focus: boolean): Promise<void> {
  if (port instanceof PanelPort) await port.focusPanel(focus);
}
