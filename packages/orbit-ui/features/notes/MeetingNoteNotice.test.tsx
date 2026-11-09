import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NoteListItem } from "../../bridge/generated/NoteListItem";
import type { NoteReadySnapshot } from "../../bridge/generated/NoteReadySnapshot";
import type { AppView } from "../../state/app-view";
import { MeetingNoteNotice } from "./MeetingNoteNotice";

// One notice per meeting, from its end to its note: «Reunión guardada»
// while the note is written (about half a minute), then «Tu nota está
// lista · Abrir», or the failure in the list's own words.

const RECORDING = "rec-comite";
const TITLE = "Comité de la biblioteca";

type Meeting = AppView["meeting"];
type Notes = NonNullable<AppView["notes"]>;

function meeting(phase: "recording" | "idle"): Meeting {
  return {
    phase,
    recording_id: RECORDING,
    recent: phase === "idle" ? [{ recording_id: RECORDING, title: TITLE }] : [],
  } as unknown as Meeting;
}

function row(change: Partial<NoteListItem> = {}): NoteListItem {
  return {
    recording_id: RECORDING,
    title: TITLE,
    started_at_ms: 0,
    duration_ms: 1_800_000,
    state: "transcribing",
    group: "today",
    ...change,
  } as NoteListItem;
}

function notes(list: NoteListItem[], ready?: NoteReadySnapshot): Notes {
  return { list, ready } as unknown as Notes;
}

const ready = (sequence: number, recording_id = RECORDING, title = TITLE): NoteReadySnapshot => ({
  recording_id,
  title,
  sequence,
});

function show(initial: { meeting: Meeting; notes?: Notes }) {
  const send = vi.fn();
  const view = render(
    <MeetingNoteNotice meeting={initial.meeting} notes={initial.notes} send={send} />,
  );
  return {
    send,
    update: (next: { meeting: Meeting; notes?: Notes }) =>
      view.rerender(<MeetingNoteNotice meeting={next.meeting} notes={next.notes} send={send} />),
  };
}

/** The notices on screen: their whole text, one each. */
function notices(): string[] {
  return [...screen.queryAllByRole("status"), ...screen.queryAllByRole("alert")].map(
    (notice) => notice.textContent ?? "",
  );
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  window.location.hash = "";
});

describe("«Reunión guardada», until its note", () => {
  it("stays past the five seconds of a confirmation while the note is written", () => {
    const view = show({ meeting: meeting("recording"), notes: notes([row()]) });
    view.update({ meeting: meeting("idle"), notes: notes([row()]) });
    expect(notices()).toHaveLength(1);
    expect(notices()[0]).toContain("Reunión guardada");
    expect(notices()[0]).toContain("Te avisamos cuando la nota esté lista.");
    act(() => vi.advanceTimersByTime(40_000));
    expect(notices()).toHaveLength(1);
    expect(notices()[0]).toContain("Reunión guardada");
    expect(screen.queryByRole("button", { name: "Abrir" })).toBeNull();
  });

  it("becomes «Tu nota está lista · Abrir» when the note is ready: one notice, never two", () => {
    const view = show({
      meeting: meeting("recording"),
      notes: notes([row()], ready(3, "rec-antes")),
    });
    view.update({ meeting: meeting("idle"), notes: notes([row()], ready(3, "rec-antes")) });
    // A short meeting's note can be ready while «Reunión guardada» is still up.
    act(() => vi.advanceTimersByTime(3_000));
    view.update({
      meeting: meeting("idle"),
      notes: notes(
        [row({ state: "ready", headline: { phase: "ready", preview: "Se aprobó" } })],
        ready(4),
      ),
    });
    expect(notices()).toHaveLength(1);
    expect(notices()[0]).toContain("Tu nota está lista");
    expect(notices()[0]).toContain(TITLE);
    expect(notices()[0]).not.toContain("Reunión guardada");
    fireEvent.click(screen.getByRole("button", { name: "Abrir" }));
    expect(window.location.hash).toBe(`#/notes/${RECORDING}`);
    expect(view.send).toHaveBeenCalledWith({ command: "notes_open", recording_id: RECORDING });
    expect(notices()).toEqual([]);
  });

  it("says the note failed, in the list's words, and stays until closed", () => {
    const view = show({ meeting: meeting("recording"), notes: notes([row()]) });
    view.update({ meeting: meeting("idle"), notes: notes([row()]) });
    view.update({
      meeting: meeting("idle"),
      notes: notes([row({ state: "ready", headline: { phase: "failed" } })]),
    });
    expect(screen.getByRole("alert").textContent).toContain("No se pudo escribir la nota");
    expect(notices()).toHaveLength(1);
    act(() => vi.advanceTimersByTime(60_000));
    expect(screen.getByRole("alert").textContent).toContain("No se pudo escribir la nota");
    fireEvent.click(screen.getByRole("button", { name: "Cerrar aviso" }));
    expect(notices()).toEqual([]);
  });

  it("says a transcription that failed as the list says it", () => {
    const view = show({ meeting: meeting("recording"), notes: notes([row()]) });
    view.update({ meeting: meeting("idle"), notes: notes([row({ state: "failed" })]) });
    expect(screen.getByRole("alert").textContent).toContain("No se pudo transcribir");
  });

  it("closed by hand, it says nothing more of that meeting", () => {
    const view = show({ meeting: meeting("recording"), notes: notes([row()]) });
    view.update({ meeting: meeting("idle"), notes: notes([row()]) });
    fireEvent.click(screen.getByRole("button", { name: "Cerrar aviso" }));
    view.update({ meeting: meeting("idle"), notes: notes([row()], ready(1)) });
    expect(notices()).toEqual([]);
  });
});

describe("«Tu nota está lista» of another note", () => {
  it("says nothing for a note that was ready before the window looked", () => {
    show({ meeting: meeting("idle"), notes: notes([], ready(4)) });
    expect(screen.queryByText("Tu nota está lista")).toBeNull();
  });

  it("tells each note that becomes ready once, with its title and «Abrir»", () => {
    const view = show({ meeting: meeting("idle"), notes: notes([], ready(4)) });
    view.update({
      meeting: meeting("idle"),
      notes: notes([], ready(5, "rec-5", "Reunión de equipo")),
    });
    expect(screen.getByText("Tu nota está lista")).toBeTruthy();
    expect(screen.getByText("Reunión de equipo")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Abrir" }));
    expect(window.location.hash).toBe("#/notes/rec-5");
    expect(view.send).toHaveBeenCalledWith({ command: "notes_open", recording_id: "rec-5" });
    expect(screen.queryByText("Tu nota está lista")).toBeNull();
  });

  it("tells the first note ready when none was before", () => {
    const view = show({ meeting: meeting("idle"), notes: notes([]) });
    expect(screen.queryByText("Tu nota está lista")).toBeNull();
    view.update({ meeting: meeting("idle"), notes: notes([], ready(1)) });
    expect(screen.getByText("Tu nota está lista")).toBeTruthy();
  });
});
