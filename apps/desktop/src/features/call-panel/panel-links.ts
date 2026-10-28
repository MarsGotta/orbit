import { getCopy } from "../../i18n";
import { reportRefused } from "../../app/report-failure";
import { panelOpenUrl } from "../../bridge/panel-port";
import type { DaemonPort } from "../../bridge/port";

/**
 * Opens a link of Deimos's answer through the panel's own order.
 * Returns what the panel says when it could not (what happened, not
 * Rust's cause), and reports the refusal with its own code.
 */
export async function openPanelLink(port: DaemonPort, url: string): Promise<string | null> {
  try {
    await panelOpenUrl(port, url);
    return null;
  } catch (error: unknown) {
    reportRefused(port, error);
    return getCopy().meetingQuestion.linkRefused;
  }
}
