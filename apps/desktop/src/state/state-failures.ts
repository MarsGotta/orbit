import type { NoticeCode } from "../bridge/generated/NoticeCode";
import type { NoticeData } from "../i18n/es/notices";
import type { AppView } from "./app-view";

/**
 * A failure the composed view shows, as the report log takes it:
 * a code of the interface's own and closed data. `key` names this failure,
 * so it is reported when it appears and not again while it stays.
 */
export interface StateFailure {
  key: string;
  code: NoticeCode;
  data: NoticeData;
}

const TOKEN = /^[a-z0-9_.-]{1,64}$/;

/** A value fit for the log, or nothing: a cause's text never travels. */
function token(raw: unknown): string | undefined {
  return typeof raw === "string" && TOKEN.test(raw) ? raw : undefined;
}

function data(entries: Partial<Record<keyof NoticeData, string | undefined>>): NoticeData {
  const kept: NoticeData = {};
  for (const [key, value] of Object.entries(entries) as [keyof NoticeData, string | undefined][])
    if (value !== undefined) kept[key] = value;
  return kept;
}

function present(text: string | null | undefined): text is string {
  return text != null && text !== "";
}

/**
 * The cause of every read the daemon's orderly closure cut off: one in flight
 * it let go of, or one asked once it had stopped. The app's channel to the
 * daemon gives that cause and no other when the daemon closed in order
 * (`LocalClient::stopped`, crates/orbit-ipc/src/ipc.rs), and the app keeps it
 * as the read's detail (pinned by `orbit-app-core/tests/topics.rs`). Orbit
 * closing is not a failed read; a daemon that fell says otherwise.
 */
const CLOSED_IN_ORDER = "orbit_closing";
const AGENDA_FAILED = "agenda_request_failed:";

function cutByClosing(detail: string | null | undefined): boolean {
  return detail === CLOSED_IN_ORDER;
}

function noteFailures(view: AppView): StateFailure[] {
  const notes = view.notes;
  const failures: StateFailure[] = [];
  const read = notes?.open_read;
  // A reread the closure cut keeps the note shown, with the cause as its detail.
  const cut = cutByClosing(read?.detail);
  if (!cut && (read?.state === "failed" || (read?.state === "available" && present(read.detail)))) {
    const stage = read.state === "failed" ? "failed" : "incomplete";
    failures.push({
      key: `note_read:${read.recording_id}:${read.revision}:${stage}`,
      code: "note_read_problem",
      data: { stage },
    });
  }
  const open = notes?.open;
  if (present(open?.transcript?.error) && !cutByClosing(open.transcript.error))
    failures.push({
      key: `transcript:${open.recording_id}`,
      code: "transcript_read_failed",
      data: {},
    });
  if (notes?.index?.state === "failed" && !cutByClosing(notes.index.detail))
    failures.push({ key: "notes_index", code: "notes_index_failed", data: {} });
  return failures;
}

function libraryFailures(view: AppView): StateFailure[] {
  const library = view.notes?.library;
  if (library == null) return [];
  const failures: StateFailure[] = [];
  const cut = library.read === "failed" && cutByClosing(library.read_detail);
  if (!cut && (library.read === "failed" || library.templates?.state === "failed"))
    failures.push({ key: "library_read", code: "library_read_failed", data: {} });
  for (const refusal of library.refusals ?? []) {
    failures.push({
      key: `library_refused:${refusal.target_kind}:${refusal.target_id}:${refusal.code}`,
      code: "library_refused",
      data: data({ code: token(refusal.code) }),
    });
  }
  return failures;
}

function updateFailures(view: AppView): StateFailure[] {
  const update = view.settings?.update;
  const failures: StateFailure[] = [];
  if (update?.state === "failed") {
    const refusal = update.refusal as { cause?: { kind?: unknown } } | undefined;
    failures.push({
      key: "update:check",
      code: "update_failed",
      data: data({ stage: "check", cause: token(refusal?.cause?.kind) }),
    });
  }
  if (present(update?.last_installation?.failure))
    failures.push({ key: "update:install", code: "update_failed", data: { stage: "install" } });
  return failures;
}

function calendarFailures(view: AppView): StateFailure[] {
  const failures: StateFailure[] = [];
  const reading = view.calendar?.reading?.detail;
  if (
    reading?.startsWith(AGENDA_FAILED) === true &&
    !cutByClosing(reading.slice(AGENDA_FAILED.length))
  )
    failures.push({ key: "agenda_read", code: "agenda_read_failed", data: {} });
  for (const write of view.calendar?.writes ?? []) {
    if (write.stage !== "refused" && write.stage !== "outcome_unknown") continue;
    failures.push({
      key: `calendar_write:${write.request_id}:${write.stage}`,
      code: "calendar_write_failed",
      data: data({ code: token(write.code), stage: write.stage }),
    });
  }
  const zone = view.system_timezone_error;
  // `timezone_io_failed:<detail>` keeps only its kind.
  if (present(zone))
    failures.push({
      key: "timezone",
      code: "timezone_failed",
      data: data({ cause: token(zone.split(":", 1)[0]) }),
    });
  return failures;
}

/**
 * Every failure the view shows whose cause the screen no longer explains.
 * `closing`: the daemon said, while this window watched, that it is closing.
 * Its reads are then cut off on purpose, and the notes and library reads that
 * fail are the quit, not a failure (real runs of 2026-10-20: the list was read
 * again 0.55–0.8 s after `orbit_closing` and a normal quit counted as one).
 */
export function stateFailures(view: AppView, closing = false): StateFailure[] {
  return [
    ...(closing ? [] : noteFailures(view)),
    ...(closing ? [] : libraryFailures(view)),
    ...updateFailures(view),
    ...calendarFailures(view),
  ];
}

/**
 * Reports each failure once, when it appears: one that stays is not said
 * again, and one that goes and comes back is a new failure. It keeps only
 * the failures on screen now, so it holds no more than the view does.
 */
export function watchFailures(): (view: AppView, closing?: boolean) => StateFailure[] {
  let shown = new Set<string>();
  return (view, closing = false) => {
    const failures = stateFailures(view, closing);
    const fresh = failures.filter((failure) => !shown.has(failure.key));
    shown = new Set(failures.map((failure) => failure.key));
    return fresh;
  };
}
