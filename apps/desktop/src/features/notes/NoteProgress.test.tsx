import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { NoteProgress } from "./NoteProgress";

afterEach(cleanup);

// Each step says what it is doing in its own words: done, in progress or
// still to come, as the person reads «Audio guardado → Transcribiendo… →
// Escribir el resumen».
it("names each step by where it stands", () => {
  render(
    <NoteProgress
      progress={{
        upload: { state: "done", cause: null },
        transcript: { state: "active", cause: { kind: "transcribing", chunks: 2 } },
        summary: { state: "waiting", cause: null },
      }}
    />,
  );
  const steps = screen.getAllByRole("listitem").map((item) => item.textContent ?? "");
  expect(steps.filter((text) => text !== "").map((text) => text.split(":")[0])).toEqual([
    "Audio guardado",
    "Transcribiendo…",
    "Escribir el resumen",
  ]);
});

it("says a finished transcript and a summary being written", () => {
  render(
    <NoteProgress
      progress={{
        upload: { state: "done", cause: null },
        transcript: { state: "done", cause: null },
        summary: { state: "active", cause: { kind: "writing" } },
      }}
    />,
  );
  expect(screen.getByText("Transcripción lista")).toBeTruthy();
  expect(screen.getByText("Escribiendo el resumen…")).toBeTruthy();
});
