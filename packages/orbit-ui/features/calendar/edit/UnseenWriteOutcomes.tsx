import { useCopy } from "../../../i18n/CopyProvider";
import { useEffect, useRef, useState } from "react";
import type { WriteOutcome } from "../../../bridge/generated/WriteOutcome";
import { Button } from "../../../design/Button";
import { Toast } from "../../../design/Toast";

import { failureNotice } from "./notices";

function failed(write: WriteOutcome): boolean {
  return write.stage === "refused" || write.stage === "outcome_unknown";
}

/**
 * A calendar write that ended refused, or with an unknown outcome, while the
 * calendar view was closed: a move or a save just before leaving it. The open
 * view says its own writes in its panel; this says, once, the ones nobody saw,
 * with the way back to the calendar.
 */
export function UnseenWriteOutcomes({
  calendarOpen,
  onOpenCalendar,
  writes,
}: {
  calendarOpen: boolean;
  onOpenCalendar: () => void;
  writes?: WriteOutcome[];
}) {
  const es = useCopy();
  // Every ended write already said, here or by the open view. The head
  // carries a few recent outcomes, so this holds no more than it does.
  const said = useRef(new Set<string>());
  const [shown, setShown] = useState<WriteOutcome | null>(null);
  useEffect(() => {
    const ended = (writes ?? []).filter(failed);
    const unseen = calendarOpen ? [] : ended.filter((write) => !said.current.has(write.request_id));
    said.current = new Set(ended.map((write) => write.request_id));
    const last = unseen.at(-1);
    if (last !== undefined) setShown(last);
  }, [writes, calendarOpen]);
  useEffect(() => {
    if (calendarOpen) setShown(null);
  }, [calendarOpen]);
  if (shown === null) return null;
  const notice = failureNotice(shown);
  return (
    <Toast
      action={
        <Button
          onClick={() => {
            setShown(null);
            onOpenCalendar();
          }}
          size={28}
          variant="outline"
        >
          {es.calendar.write.openCalendar}
        </Button>
      }
      detail={notice.text}
      key={shown.request_id}
      kind="failed"
      message={
        notice.kind === "unknown"
          ? es.calendar.write.unseenUnknown
          : es.calendar.write.unseenRefused
      }
      onClose={() => setShown(null)}
    />
  );
}
