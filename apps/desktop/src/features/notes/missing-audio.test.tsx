import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { MissingAudio, MutedMicrophone } from "./TranscriptTools";

afterEach(cleanup);

function note(): OpenNoteSnapshot {
  return {
    recording_id: "reunion-con-hueco",
    title: "Reunión sintética",
    capture: "closed",
    owner_name: "Alina",
    started_at_ms: 1_790_500_000_000,
    ended_at_ms: 1_790_500_120_000,
    event: null,
    summary: null,
    note_revision: null,
    actions: [],
    decisions: [],
    open_topics: [],
    private_notes: "",
    private_notes_revision: 0,
    progress: {
      upload: { state: "done", cause: null },
      transcript: { state: "done", cause: null },
      summary: { state: "waiting", cause: { kind: "not_requested" } },
    },
    transcript: {
      language: "es",
      turns: [
        { index: 0, at_ms: 0, speaker_key: "mic_local", text: "Empezamos." },
        { index: 1, at_ms: 12_000, speaker_key: "remote_app", text: "Vale." },
        { index: 2, at_ms: 3_725_000, speaker_key: "mic_local", text: "Cerramos." },
      ],
      pending_chunks: 0,
      failed_groups: 0,
    },
    speakers: [
      {
        key: "mic_local",
        track: "mic_local",
        name: "Alina",
        color: "var(--o-c2)",
        share_pct: 67,
      },
      {
        key: "remote_app",
        track: "remote_app",
        name: "Otra persona",
        color: "var(--o-c3)",
        share_pct: 33,
      },
    ],
  };
}

describe("a transcript with audio the hub never got", () => {
  it("says which part is missing, from when and for how long", () => {
    const shown = note();
    shown.transcript.missing_audio = [{ track: "mic_local", at_ms: 125_000, duration_ms: 1_000 }];
    render(<MissingAudio note={shown} />);
    expect(
      screen.getByText(
        "Falta el audio del micrófono desde 02:05 durante 1 s: no llegó al hub y esa parte no está en la transcripción.",
      ),
    ).toBeTruthy();
  });

  it("says it without a time when the hub could not tell", () => {
    const shown = note();
    shown.transcript.missing_audio = [{ track: "remote_app", at_ms: null, duration_ms: null }];
    render(<MissingAudio note={shown} />);
    expect(
      screen.getByText(
        "Falta el audio de la reunión: no llegó al hub y esa parte no está en la transcripción.",
      ),
    ).toBeTruthy();
  });
});

describe("a transcript with the microphone muted", () => {
  it("says each muted stretch in one calm line, from when to when", () => {
    const shown = note();
    shown.transcript.muted_microphone = [
      { start_ms: 724_000, end_ms: 750_000 },
      { start_ms: 3_600_000, end_ms: 3_605_000 },
    ];
    render(<MutedMicrophone note={shown} />);
    expect(screen.getByText("Micrófono silenciado · 12:04 – 12:30")).toBeTruthy();
    expect(screen.getByText("Micrófono silenciado · 60:00 – 60:05")).toBeTruthy();
  });

  it("says nothing for a meeting never muted", () => {
    const { container } = render(<MutedMicrophone note={note()} />);
    expect(container.textContent).toBe("");
  });
});
