import type { AppView } from "../state/app-view";
import type { BriefingSnapshot } from "../bridge/generated/BriefingSnapshot";
import { BriefingBody } from "../features/briefing/BriefingBody";
import { appView, notesSnapshot } from "../features/notes/test-notes";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { configureCopy, getCopy } from ".";
import { PreviousActions } from "../features/briefing/PreviousActions";

afterEach(() => {
  cleanup();
  configureCopy("es", ["es-ES"], "Atlas");
});

it.each(["es", "en"] as const)("renders briefing codes and numeric facts in %s", (language) => {
  configureCopy(language, [language], "Atlas");
  const cause = getCopy().briefingCause;
  expect(cause("topic_not_observed")).toBe(
    language === "es" ? "Este estado aún no se ha leído." : "This state hasn’t been read yet.",
  );
  expect(cause(JSON.stringify({ code: "topic_stale", since_ms: 12345 }))).toContain(
    language === "es" ? "12.345" : "12,345",
  );
  expect(
    cause(
      JSON.stringify({ code: "transcription", completed: 2, pending: 1, failed: 0, queued: true }),
    ),
  ).toBe(
    language === "es"
      ? "Transcripción: 2 completadas, 1 pendiente, 0 fallidas; hay otra en cola."
      : "Transcription: 2 completed, 1 pending, 0 failed; another is queued.",
  );
  expect(
    cause(
      JSON.stringify({
        code: "agenda_window_excludes_today",
        window_start_ms: 10000,
        window_end_ms: 20000,
        today_start_ms: 30000,
        today_end_ms: 40000,
      }),
    ),
  ).toContain(language === "es" ? "10.000" : "10,000");
  expect(cause(JSON.stringify({ code: "reading_since", since_ms: 12345 }))).toBe(
    language === "es"
      ? "La lectura está en curso desde 12.345 ms Unix."
      : "The read has been in progress since 12,345 ms Unix.",
  );
  const refusal = JSON.stringify({
    code: "internal",
    cause: {
      kind: "app_operation",
      code: "read_calendars_count",
      error: JSON.stringify({ count: 2, total: 3 }),
    },
  });
  const counts =
    language === "es"
      ? "Se leyeron 2 de 3 calendarios anunciados."
      : "Read 2 of 3 advertised calendars.";
  expect(cause(refusal)).toBe(counts);
  expect(cause(`agenda_request_failed:${refusal}`)).toBe(counts);
  const entryFailure = JSON.stringify({
    code: "internal",
    cause: {
      kind: "app_operation",
      code: "read_calendars_invalid",
      error: JSON.stringify({ index: 12345, diagnostic: "private" }),
    },
  });
  expect(cause(entryFailure)).toBe(
    language === "es"
      ? "El calendario de la posición 12.345 no tiene un formato válido."
      : "Calendar entry 12,345 has an invalid format.",
  );
  const malformed = JSON.stringify({
    code: "transcription",
    completed: { valueOf: "private", toString: "private" },
    pending: 1,
    failed: 0,
    queued: false,
  });
  expect(() => cause(malformed)).not.toThrow();
  expect(cause(malformed)).not.toContain("private");
  expect(cause("private diagnostic in an old snapshot")).not.toContain("private diagnostic");
});

it.each(["es", "en"] as const)("localizes previous-action read failures in %s", (language) => {
  configureCopy(language, [language], "Atlas");
  render(
    <PreviousActions
      actions={{
        state: "unknown",
        cause: JSON.stringify({
          code: "internal",
          cause: { kind: "app_operation", code: "read_series_empty", error: "private diagnostic" },
        }),
      }}
      send={vi.fn()}
    />,
  );
  expect(
    screen.getByText(
      language === "es" ? "La serie de la reunión llegó vacía." : "The meeting series was empty.",
    ),
  ).toBeTruthy();
  expect(document.body.textContent).not.toContain("private diagnostic");
});

it.each(["es", "en"] as const)(
  "localizes the agenda failure shown by the briefing in %s",
  (language) => {
    configureCopy(language, [language], "Atlas");
    const snapshot = {
      ...appView(notesSnapshot()),
      meeting: {
        phase: "idle",
        recording_id: null,
        title: null,
        started_at_ms: null,
        ends_at_ms: null,
        limit_minutes: 0,
        audio: "system",
        tracks: [],
        upload: { pending_chunks: 0, offline: false },
        call_panel: null,
        call_preference: null,
        recent: [],
        applications: [],
        applications_state: "empty",
      } satisfies AppView["meeting"],
    };
    const briefing: BriefingSnapshot = {
      greeting: { name: "Robin" },
      summary: { meetings_today: 0, focus_minutes: 0 },
      agenda: [],
      next: null,
      sources: {
        agenda: { state: "absent", cause: "notes_list_failed" },
        notes: { state: "present" },
        recording: { state: "present" },
      },
    };
    render(
      <BriefingBody
        briefing={briefing}
        snapshot={{
          ...snapshot,
          briefing,
          calendar: {
            ...snapshot.calendar,
            authorization: "granted",
            reading: { state: "unavailable" },
          },
        }}
        nowMs={0}
        onRecord={vi.fn()}
        send={vi.fn()}
        timezone="UTC"
      />,
    );
    expect(screen.getByRole("alert").textContent).toContain(
      language === "es"
        ? "No se pudo leer la lista de reuniones."
        : "Couldn’t read the meeting list.",
    );
    expect(screen.getByRole("alert").textContent).not.toContain("notes_list_failed");
  },
);
