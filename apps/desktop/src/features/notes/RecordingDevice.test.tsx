import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ShellSlots, useShellSlots } from "../../app/shell-slots";
import { MeetingsColumn } from "./MeetingsColumn";
import { NoteDetail } from "./NoteDetail";
import { appView, notesSnapshot, writtenNote } from "./test-notes";

afterEach(cleanup);
const send = vi.fn(() => Promise.resolve());
const recorded_on = { device_id: "recording-mac", name: "Mac de Alba" };

function ColumnHost() {
  return <div ref={useShellSlots()?.setColumn} />;
}

it("names the recording Mac in the list and open meeting", () => {
  const notes = notesSnapshot();
  Object.assign(notes.list[0]!, { recorded_on });
  const { unmount } = render(
    <ShellSlots>
      <ColumnHost />
      <MeetingsColumn send={send} snapshot={appView(notes)} />
    </ShellSlots>,
  );
  expect(screen.getByText("Grabada en Mac de Alba")).toBeTruthy();
  unmount();
  const note = Object.assign(writtenNote(), { recorded_on });
  render(<NoteDetail note={note} send={send} error={null} />);
  expect(screen.getByText(/Grabada en Mac de Alba/)).toBeTruthy();
});

it("explains where pending audio must be uploaded without offering a failing action", () => {
  const note = Object.assign(writtenNote(), { recorded_on, capture: "open" as const });
  render(<NoteDetail note={note} send={send} error={null} />);
  expect(
    screen.getByText(
      "El audio pendiente debe subirse desde Mac de Alba. Mantén Orbit abierto allí hasta que termine.",
    ),
  ).toBeTruthy();
  fireEvent.click(screen.getByRole("tab", { name: "Transcripción" }));
  expect(screen.queryByRole("button", { name: "Rehacer transcripción" })).toBeNull();
  expect(send).not.toHaveBeenCalled();
});

it("keeps a single-Mac meeting free of a device label", () => {
  render(<NoteDetail note={writtenNote()} send={send} error={null} />);
  expect(screen.queryByText(/Grabada en/)).toBeNull();
});
