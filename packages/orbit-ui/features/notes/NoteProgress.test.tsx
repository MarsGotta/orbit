import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import type { NoteProgressSnapshot } from "../../bridge/generated/NoteProgressSnapshot";
import { NoteProgress } from "./NoteProgress";
import { writingNote } from "./test-notes";

afterEach(cleanup);

function card(progress: NoteProgressSnapshot) {
  render(<NoteProgress note={{ ...writingNote(), progress }} send={() => undefined} />);
}

// Each step says what it is doing in its own words: done, in progress or
// still to come, as the person reads «Audio guardado → Transcribiendo… →
// Escribir el resumen».
it("names each step by where it stands", () => {
  card({
    upload: { state: "done", cause: null },
    transcript: { state: "active", cause: { kind: "transcribing", chunks: 2 } },
    summary: { state: "waiting", cause: null },
  });
  expect(screen.getAllByRole("listitem").map((item) => item.textContent)).toEqual([
    "Audio guardado",
    "Transcribiendo…",
    "Escribir el resumen",
  ]);
});

it("says a finished transcript and a summary being written", () => {
  card({
    upload: { state: "done", cause: null },
    transcript: { state: "done", cause: null },
    summary: { state: "active", cause: { kind: "writing" } },
  });
  expect(screen.getByText("Transcripción lista")).toBeTruthy();
  expect(screen.getByText("Escribiendo el resumen…")).toBeTruthy();
});
