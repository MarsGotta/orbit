/**
 * How long the interface waits for something before it calls it a failure.
 *
 * Each ceiling here sits on top of a bound the app already has, so the
 * bounded part's own refusal, which names its cause, reaches the screen first;
 * the interface's ceiling only ends a wait that nothing else would end. None
 * of them is a retry: past one, the order is never sent again, and a late
 * answer is not shown as the order's.
 *
 * The numbers marked «copy» are bounds of the daemon or the app, registered
 * in `contracts/constants.json` and compared with their source by
 * `scripts/tests/test_constants_copies.py`.
 */
import type { AppCommand, FeatureId } from "../bridge/generated";

/** Copy: one call to the daemon, its queue included (`CALL_TIMEOUT_SECONDS`, orbit-ipc). */
export const CALL_TIMEOUT_SECONDS = 10;
/** Copy: one attachment call, upload or read (`ATTACHMENT_CALL_TIMEOUT_SECONDS`, orbit-ipc). */
export const ATTACHMENT_CALL_TIMEOUT_SECONDS = 100;
/** Copy: one request from the daemon to the hub (`HUB_HTTP_TIMEOUT_SECONDS`, orbit-daemon). */
export const HUB_HTTP_TIMEOUT_SECONDS = 90;
/** Copy: the hub's ceiling for one voice request (`HUB_VOICE_DEADLINE_SECONDS`, orbit-daemon). */
export const HUB_VOICE_DEADLINE_SECONDS = 240;
/** Copy: pages one refresh of the notes list reads (`MAX_LIST_PAGES`, orbit-app-core). */
export const MAX_LIST_PAGES = 20;
/** Copy: pages of turns one note read reads after its document (`MAX_TURN_PAGES`, orbit-app-core). */
export const MAX_TURN_PAGES = 64;
/** Copy: one download of the updater (`DOWNLOAD_TIMEOUT_SECONDS`, orbit-app-core). */
export const DOWNLOAD_TIMEOUT_SECONDS = 60;
/** Copy: the agenda's confirmation of a period it was asked to read (`READ_WAIT_SECONDS`, orbit-app-core). */
export const READ_WAIT_SECONDS = 10;

const SECOND_MS = 1_000;
const CALL_MS = CALL_TIMEOUT_SECONDS * SECOND_MS;

/**
 * Delivering to the screen what the app has already decided: a receipt that
 * left the daemon, or the frames it published before answering. That travels
 * inside the app, with no queue, hub or helper on the way, and the app bounds
 * a whole call to the daemon, queue included, at 10 s. So 10 s more: a frame
 * later than that is lost, not slow. It protects the screen from a wait no
 * frame will end; it does not limit how long the order itself works.
 */
export const DELIVERY_MS = CALL_MS;

/**
 * What the app may spend before an order's receipt, beyond one daemon call,
 * for the orders it resolves itself. Each is the bound of the work done
 * before the receipt:
 *
 * - the notes list: one call per page, up to `MAX_LIST_PAGES`;
 * - opening a note: its document and up to `MAX_TURN_PAGES` pages of turns;
 * - the template library: the order, then the whole library read again. That
 *   read has no page cap of its own (only a repeated cursor ends it), so it is
 *   given the list's page budget; a longer one is reported as exceeded and
 *   still lands when it ends;
 * - the calendar: the call, then the agenda's confirmation of the period
 *   (`READ_WAIT_SECONDS`);
 * - the update check: one download; installing: two downloads and the
 *   installer's 5 s readiness, rounded up to a third download.
 *
 * Orders macOS resolves (a link, System Settings, the login item) have no
 * bound of their own; they get one call's: a local system call is no slower
 * than an order that crosses the daemon's queue.
 */
const RECEIPT_MS: Partial<Record<AppCommand["command"], number>> = {
  notes_refresh: MAX_LIST_PAGES * CALL_MS,
  notes_more: MAX_LIST_PAGES * CALL_MS,
  notes_open: (1 + MAX_TURN_PAGES) * CALL_MS,
  note_library: CALL_MS + MAX_LIST_PAGES * CALL_MS,
  calendar: CALL_MS + READ_WAIT_SECONDS * SECOND_MS,
  calendar_refresh: CALL_MS + READ_WAIT_SECONDS * SECOND_MS,
  calendar_set_range: CALL_MS + READ_WAIT_SECONDS * SECOND_MS,
  calendar_show_today: CALL_MS + READ_WAIT_SECONDS * SECOND_MS,
  update_check: DOWNLOAD_TIMEOUT_SECONDS * SECOND_MS,
  update_install: 3 * DOWNLOAD_TIMEOUT_SECONDS * SECOND_MS,
};

/** From the moment an order leaves until the screen shows its receipt's frames. */
export function orderCeilingMs(command: AppCommand["command"]): number {
  return (RECEIPT_MS[command] ?? CALL_MS) + DELIVERY_MS;
}

/** The calls a view makes to the app directly, outside `send`. */
export type DirectCall =
  | "attachment_add"
  | "attachment_content"
  | "attachment_open"
  | "sessions_page"
  | "session_messages"
  | "agent";

/**
 * A direct call's ceiling: its own bound in the app and its delivery. An
 * attachment call is bounded at `ATTACHMENT_CALL_TIMEOUT_SECONDS`; opening
 * one also asks macOS to open the file it wrote (one call more). Reading
 * conversations, and an agent's configuration, are one daemon call each.
 */
export function callCeilingMs(call: DirectCall): number {
  const attachment = ATTACHMENT_CALL_TIMEOUT_SECONDS * SECOND_MS;
  const own: Record<DirectCall, number> = {
    attachment_add: attachment,
    attachment_content: attachment,
    attachment_open: attachment + CALL_MS,
    sessions_page: CALL_MS,
    session_messages: CALL_MS,
    agent: CALL_MS,
  };
  return own[call] + DELIVERY_MS;
}

/**
 * Connecting the window: listening to the frames and reading the first view,
 * both local to the app and answered from what it already holds. One call's
 * bound, as for the orders macOS resolves.
 */
export const CONNECT_CEILING_MS = CALL_MS;

/**
 * The first state of a feature after the window connected. The daemon
 * publishes every topic once its recovery ends, and nothing in it bounds that
 * recovery. Measured: a recovery took 36.4 s at worst on a loaded disk
 * (`crates/orbit-app-core/tests/topics.rs`) and a cold daemon reached a state
 * in 2.6 s at worst, p99 1.6 s (`tests/daemon_client.rs`). Twice the worst
 * recovery, rounded up: a first state later than that is not a slow start but
 * one that is not coming (a recovery that jammed, a helper that never said it
 * is ready). It protects a view from waiting without end; the state is still
 * taken, and the failure withdrawn, when it arrives.
 */
export const FIRST_STATE_CEILING_MS = 75_000;

/**
 * A working message the daemon said, which only a later code of its op ends.
 * «Transcribiendo tu voz…» is said as the voice note leaves for the hub, whose
 * request is bounded at `HUB_VOICE_DEADLINE_SECONDS` plus the 10 s connection
 * (`VOICE_HUB_HTTP_TIMEOUT`); its outcome, failure included, arrives with its
 * own code. Past that and its delivery, the message only says something that
 * already ended (a turn that began without naming the op), so it goes. Other
 * working messages end with the process (closing) and have no ceiling here.
 */
export const WORKING_CEILING_MS: Partial<Record<string, number>> = {
  voice_transcribing: (HUB_VOICE_DEADLINE_SECONDS + CALL_TIMEOUT_SECONDS) * SECOND_MS + DELIVERY_MS,
};

/**
 * A choice the hub confirms (a template, a language, a task's box, an answer
 * to Deimos): it is kept on screen as chosen after its order was accepted
 * until the hub's change arrives. The daemon's request to the hub is bounded
 * at `HUB_HTTP_TIMEOUT_SECONDS`; its refusal arrives as its own message. Past
 * that, the call and the delivery, what the screen shows is what is known
 * again. It protects a control from showing a choice nobody is confirming.
 */
export const HUB_CONFIRMATION_MS = HUB_HTTP_TIMEOUT_SECONDS * SECOND_MS + CALL_MS + DELIVERY_MS;

/** The features whose first state has a ceiling: those that say when they have none. */
export const FIRST_STATE_FEATURES: readonly FeatureId[] = [
  "status",
  "permissions",
  "notes",
  "recording",
  "assistant",
  "control",
];
