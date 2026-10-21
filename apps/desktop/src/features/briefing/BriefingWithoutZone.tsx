import type { AppView as AppSnapshot } from "../../state/app-view";
import type { BriefingSnapshot } from "../../bridge/generated/BriefingSnapshot";
import { Button } from "../../design/Button";
import { ViewError } from "../../design/ViewError";
import { es } from "../../i18n/es";
import { BriefingSections } from "./BriefingSections";
import styles from "./BriefingView.module.css";

interface Props {
  briefing: BriefingSnapshot;
  error: string | null;
  onRecord: () => void;
  snapshot: AppSnapshot;
}

/**
 * Without the Mac's zone no hour can be placed, so no meeting is shown. A
 * zone on its way says nothing; one the system could not give says so.
 */
export function BriefingWithoutZone({ briefing, error, onRecord, snapshot }: Props) {
  const untimedBriefing: BriefingSnapshot = {
    ...briefing,
    next: null,
    upcoming: [],
    agenda: [],
    sources: briefing.sources && {
      ...briefing.sources,
      agenda:
        briefing.sources.agenda.state === "present"
          ? { state: "unknown", cause: "local_zone_unavailable" }
          : briefing.sources.agenda,
    },
  };
  const untimedSnapshot = { ...snapshot, briefing: untimedBriefing };
  return (
    <div className={styles.page}>
      <ViewError error={error} />
      <div className={styles.content}>
        <header className={styles.hero}>
          <h1>{es.briefing.hello}</h1>
          {Boolean(snapshot.system_timezone_error) && <p>{es.briefing.timezoneFailed}</p>}
          <Button onClick={onRecord} variant="accent">
            {es.meeting.recordNow}
          </Button>
        </header>
        <div className={styles.grid}>
          <BriefingSections briefing={untimedBriefing} snapshot={untimedSnapshot} />
        </div>
      </div>
    </div>
  );
}
