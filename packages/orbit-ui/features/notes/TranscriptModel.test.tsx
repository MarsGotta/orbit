import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { TranscriptTab } from "./TranscriptTab";
import { writtenNote } from "./test-notes";

afterEach(cleanup);
it("offers the other model and sends the choice with the retranscription", async () => {
  const note = writtenNote();
  note.transcript.final_pass = { state: "completed", error: null, model: "whisper" };
  const send = vi.fn(() => Promise.resolve());
  render(<TranscriptTab note={note} send={send} />);
  fireEvent.click(screen.getByRole("button", { name: "Rehacer transcripción" }));
  const choice = screen.getByRole("combobox", { name: "Volver a transcribir con…" });
  expect((choice as HTMLSelectElement).value).toBe("mai");
  expect(screen.getByText(/Microsoft a través de OpenRouter/)).toBeTruthy();
  fireEvent.click(screen.getAllByRole("button", { name: "Rehacer transcripción" }).at(-1)!);
  await waitFor(() =>
    expect(send).toHaveBeenCalledWith({
      command: "transcript_retranscribe",
      recording_id: note.recording_id,
      model: "mai",
    }),
  );
});
