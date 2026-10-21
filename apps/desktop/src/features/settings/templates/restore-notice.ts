import type { DaemonConnection } from "../../../bridge/generated/DaemonConnection";
import type { NoteLibrarySnapshot } from "../../../bridge/generated/NoteLibrarySnapshot";
import { es } from "../../../i18n/es";
import type { RemovedTemplate } from "./template-draft";

/**
 * «Deshacer» pressed. `requestId` names it to the daemon, which
 * answers it in the library (`restores`); `settled` once the order was
 * answered; `errorBefore`, the view's refusal then, which is not this one's.
 */
export interface Restoring {
  removed: RemovedTemplate;
  requestId: string;
  settled: boolean;
  errorBefore: string | null;
  /** The connection with the daemon was not ready at some point after the
   *  press: a restart loses the answers it kept in memory. */
  interrupted: boolean;
}

export interface RestoreNotice {
  kind: "status" | "alert";
  text: string;
}

type Library = NoteLibrarySnapshot | null | undefined;

/**
 * Where an undo stands, from what the library shows after the daemon took
 * it — never from the press, nor from a frame older than its answer: on its
 * way until the template is back without `pending` and each series the
 * daemon gave back is linked again or refused; refused when the Mac or the
 * hub refused it (its cause stays in the log).
 */
export function restoreNotice(
  restoring: Restoring,
  library: Library,
  error: string | null,
  daemon: DaemonConnection = "ready",
): RestoreNotice {
  const { name, id, series } = restoring.removed;
  const refused = library?.refusals.find(
    (refusal) => refusal.target_kind === "template" && refusal.target_id === id,
  );
  if (refused) return { kind: "alert", text: es.library.restoreRefused(name) };
  const shown = library?.items.find((item) => item.id === id);
  const answer = library?.restores.find((restore) => restore.request_id === restoring.requestId);
  if (answer === undefined && restoring.interrupted)
    return withoutAnswer(name, daemon === "ready" ? shown : { pending: true });
  if (answer === undefined) {
    const newError = error !== null && error !== restoring.errorBefore;
    return restoring.settled && newError && shown === undefined
      ? { kind: "alert", text: es.library.restoreRefused(name) }
      : { kind: "status", text: es.library.restoring(name) };
  }
  if (shown === undefined || shown.pending)
    return { kind: "status", text: es.library.restoring(name) };
  if (series === null) return { kind: "status", text: es.library.restored(name) };
  const refusedSeries = seriesRefused(series, id, library);
  if (refusedSeries === null) return { kind: "status", text: es.library.restoring(name) };
  return refusedSeries > 0
    ? { kind: "alert", text: es.library.restoredSomeSeriesNot(name) }
    : { kind: "status", text: es.library.restored(name) };
}

/**
 * The daemon restarted before it answered: the list, once it is connected
 * again, is all that is known. A template still owed is in the durable
 * outbox and on its way; one not shown may or may not come back.
 */
function withoutAnswer(name: string, shown: { pending: boolean } | undefined): RestoreNotice {
  if (shown === undefined)
    return { kind: "status", text: es.library.restoreUnknownAfterRestart(name) };
  if (shown.pending) return { kind: "status", text: es.library.restoring(name) };
  return { kind: "status", text: es.library.restored(name) };
}

/**
 * How many series given back were refused, or `null` while one is still on
 * its way.
 */
function seriesRefused(series: string[], id: string, library: Library): number | null {
  let refused = 0;
  for (const key of series) {
    const linked = library?.assignments.some(
      (link) => link.calendar_item_id === key && link.template_id === id && !link.pending,
    );
    if (linked === true) continue;
    const refusal = library?.refusals.some(
      (item) => item.target_kind === "series" && item.target_id === key,
    );
    if (refusal !== true) return null;
    refused += 1;
  }
  return refused;
}
