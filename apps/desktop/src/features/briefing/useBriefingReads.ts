import { useEffect } from "react";
import type { AppCommand } from "../../bridge/generated";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";

/**
 * What the Briefing asks for when it opens (P9-20): the Briefing is about
 * today. When the agenda read does not hold today (the calendar was left on a far period, or the day changed), it asks
 * app-core to read today again, which returns to today in the same view when
 * it has to. «agenda_window_excludes_today» is then only a passing state.
 */
export function useBriefingReads(
  briefing: BriefingSnapshot | null | undefined,
  send: (command: AppCommand) => void,
) {
  const agenda = briefing?.sources?.agenda;
  const excluded =
    agenda?.state === "unknown" && agenda.cause.startsWith("agenda_window_excludes_today");
  useEffect(() => {
    if (excluded) send({ command: "calendar_show_today" });
  }, [excluded, send]);
}
