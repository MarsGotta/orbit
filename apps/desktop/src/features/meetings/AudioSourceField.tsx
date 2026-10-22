import type { MeetingApplicationSnapshot } from "../../bridge/generated/MeetingApplicationSnapshot";
import { Button } from "../../design/Button";
import { RadioList } from "../../design/RadioList";
import { Loading } from "../../design/Spinner";
import { es } from "../../i18n/es";
import type { AudioSource } from "./start-options";
import styles from "./MeetingStartDialog.module.css";

/**
 * What is known of the applications that play audio: the list, none, the
 * Mac still looking (just after the dialog asked), or no answer.
 */
export type ApplicationsKnown = "ready" | "searching" | "empty" | "unknown";

export interface AudioSourceFieldProps {
  source: AudioSource;
  onSource: (source: AudioSource) => void;
  applications: MeetingApplicationSnapshot[];
  known: ApplicationsKnown;
  /** The application chosen among `applications`, if any. */
  chosen: MeetingApplicationSnapshot | null;
  onChoose: (pid: number) => void;
  onSearchAgain: () => void;
  /** The id of what says why no application can be recorded yet. */
  statusId: string;
}

/** «Qué audio grabar»: three sources, each explained; an application by its chip. */
export function AudioSourceField(props: AudioSourceFieldProps) {
  const { source, onSource, known } = props;
  const options = [
    { value: "system", label: es.meeting.sourceSystem, help: es.meeting.sourceSystemHelp },
    {
      value: "microphone",
      label: es.meeting.sourceMicrophone,
      help: es.meeting.sourceMicrophoneHelp,
    },
    {
      value: "application",
      label: es.meeting.sourceApplication,
      // While the list arrives the option says so; chosen, the line under the list does.
      help:
        known === "searching" && source !== "application"
          ? es.meeting.sourceSearching
          : es.meeting.sourceApplicationHelp,
    },
  ] as const;
  return (
    <fieldset className={styles.fieldset}>
      <legend className={styles.label}>{es.meeting.sourceLabel}</legend>
      <RadioList
        label={es.meeting.sourceLabel}
        name="meeting-audio"
        onChange={onSource}
        options={options}
        value={source}
      />
      {source === "application" && <Applications {...props} />}
    </fieldset>
  );
}

function Applications({
  applications,
  known,
  chosen,
  onChoose,
  onSearchAgain,
  statusId,
}: AudioSourceFieldProps) {
  if (known === "searching") {
    return (
      <div className={styles.below} id={statusId}>
        <Loading size={12}>{es.meeting.sourceSearching}</Loading>
      </div>
    );
  }
  if (known !== "ready") {
    return (
      <div className={styles.below}>
        <span className={styles.meta} id={statusId}>
          {known === "empty" ? es.meeting.sourceNoApplications : es.meeting.sourceUnknown}
        </span>
        <Button onClick={onSearchAgain} size={28} variant="outline">
          {es.meeting.searchAgain}
        </Button>
      </div>
    );
  }
  return (
    <>
      <div className={styles.chips}>
        <span className={styles.meta}>{es.meeting.playingNow}</span>
        {applications.map((application) => (
          <button
            aria-pressed={application.pid === chosen?.pid}
            className={styles.chip}
            key={application.pid}
            onClick={() => onChoose(application.pid)}
            type="button"
          >
            {application.name}
          </button>
        ))}
      </div>
      <p className={styles.meta}>{es.meeting.wholeApplication}</p>
    </>
  );
}
