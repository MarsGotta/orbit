import type { AppCommand } from "../../bridge/generated";
import type { NoticeCode } from "../../bridge/generated/NoticeCode";
import type { noticeClass } from "../../bridge/generated/noticeClasses";

/** The notice codes Rust classes as a failure the person cannot fix. */
export type FailedNoticeCode = {
  [P in NoticeCode]: (typeof noticeClass)[P] extends "failed" ? P : never;
}[NoticeCode];

/**
 * What could not be done, as it ends «No se pudo …»: one phrase per order,
 * so a refusal says which order failed. The compiler asks for a new order's.
 */
const commands: Record<AppCommand["command"], string> = {
  approval: "send your answer",
  ask_orbit_about: "ask about this meeting",
  assistant_folder_add: "add the folder",
  assistant_folder_remove: "remove the folder",
  audio_sources_refresh: "list available recording sources",
  calendar: "save the calendar change",
  calendar_refresh: "refresh the calendar",
  calendar_set_range: "load those calendar dates",
  calendar_show_today: "show today in the calendar",
  call_dismiss: "dismiss the call notice",
  call_preference: "save what happens when a video call starts",
  call_record: "record the call",
  call_start: "start the call",
  call_stop: "hang up",
  call_keep: "continue the call",
  chat: "send the message",
  clarify: "send your answer",
  language_set: "change the language",
  error_reports: "change error reports",
  handover: "continue with another assistant",
  interrupt: "stop the response",
  login_item: "change whether Orbit opens at login",
  meeting_ask: "send the question",
  meeting_delete: "delete the meeting",
  meeting_discard: "end without saving",
  meeting_finish: "end the recording",
  meeting_keep: "keep recording",
  meeting_language: "change the language",
  meeting_pause: "pause the recording",
  meeting_question_close: "close the question",
  meeting_question_hold: "keep the answer in the panel",
  meeting_rename: "change the title",
  meeting_resume: "keep recording",
  microphone_mute: "mute or unmute the microphone",
  meeting_start: "start recording",
  new_session: "open a new conversation",
  note_action_toggle: "save the action",
  note_edit: "save the note",
  note_library: "save the template or dictionary",
  note_private_save: "save your private note",
  note_regenerate: "redo the summary",
  note_restore_superseded: "restore the previous note version",
  notes_more: "load more meetings",
  notes_open: "open the note",
  notes_refresh: "refresh your meetings",
  onboarding_done: "finish setup",
  open_system_privacy: "open System Settings",
  open_url: "open the link",
  pair: "pair this Mac",
  permissions: "request permission",
  privacy_stop: "stop everything",
  profile_set: "save your name",
  quit: "quit Orbit",
  reminder_read: "read the reminder from the hub",
  reminder_save: "save the reminder on the hub",
  reminder_write: "save the reminder on the hub",
  rearm: "rearm Orbit",
  recording_keep: "keep recording the call",
  restart: "restart Orbit",
  session_open: "open the conversation",
  speak: "read the answer aloud",
  speak_stop: "stop reading aloud",
  speaker_rename: "change the speaker",
  transcript_retranscribe: "redo the transcript",
  transcript_retry: "transcribe the missing audio",
  update_check: "check for updates",
  update_install: "install the update",
  voice_cancel: "cancel the voice note",
  voice_discard: "discard the voice note",
  voice_end: "send the voice note",
  voice_note_audio: "play the voice note",
  voice_retry: "send the voice note again",
  voice_start: "start the voice note",
};

/** The same, for the daemon's failures; `unknown` is a code this build does not know. */
const notices: Record<FailedNoticeCode, string> = {
  recording_close_unconfirmed: "confirm the recording ended",
  recording_interrupted_by_restart: "resume recording after Orbit restarts",
  capture_refused: "start recording",
  capture_cut: "keep recording",
  recording_seal_failed: "close the recording; the audio is saved",
  audio_stop_unconfirmed: "confirm audio stopped",
  audio_start_refused: "start audio",
  microphone_mute_refused: "mute or unmute the microphone",
  mac_command_refused: "perform the requested action on this Mac",
  mac_command_degraded: "complete the requested action on this Mac",
  helper_error: "finish a task on this Mac",
  shortcut_refused: "perform the shortcut’s action",
  voice_note_refused: "send the voice note",
  assistant_failed: "finish the response",
  assistant_interrupt_unconfirmed: "confirm the response stopped",
  approval_outcome_unknown: "check whether your answer was applied",
  call_stopped_by_connection: "keep the call open",
  call_drain_unconfirmed: "confirm the call ended",
  call_detection_failed: "detect the video call",
  hub_error: "complete the hub request",
  hub_device_identity_missing: "confirm this Mac with the hub",
  stop_step_failed: "stop everything completely",
  contained_failure: "finish a task",
  meeting_question_not_sent: "send the question",
  speech_failed: "read the answer aloud",
  store_problem: "save data on this Mac",
  closed_with_pending_work: "finish saving before closing",
  hub_event_failed: "process a hub message",
  work_failed: "finish a task",
  store_recovery_lost_files: "recover all files",
  source_failed: "read an audio source",
  note_change_refused: "save the note change",
  upload_refused: "upload audio to the hub; it stays on this Mac",
  unknown: "complete the action",
};

/**
 * The generic failure (design guide 3, «Fallos»): what could not be done,
 * what to do, and the code to share. No «administrador»: the person runs
 * their own Orbit, and whoever runs their hub reads the code in its log.
 */
const helpBefore = "Try again. If it happens again, share code ";
const helpAfter = " with whoever manages your hub.";

export const failures = {
  failure: {
    title: (what: string) => `Couldn’t ${what}`,
    help: (code: string) => `${helpBefore}${code}${helpAfter}`,
    // The help around its code, which the notice sets apart to be read aloud.
    helpBefore,
    helpAfter,
    fallback: "complete the action",
    renderFailed: "show this view",
    retry: "Try again",
    copyCode: "Copy code",
    copying: "Copying…",
    copied: "✓ Copied",
    commands,
    notices,
  },
} as const;
