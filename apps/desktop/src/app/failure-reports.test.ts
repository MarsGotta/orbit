import { describe, expect, it, vi } from "vitest";
import type { FeatureFrame } from "../bridge/generated";
import type { DaemonPort, Report } from "../bridge/port";
import type { AppView } from "../state/app-view";
import { createOrbitStore } from "../state/orbit-store";
import { stateFailures, watchFailures } from "../state/state-failures";
import { openPanelLink } from "../features/call-panel/panel-links";
import { es } from "../i18n/es";
import { reportingReads } from "./report-failure";

// A cause taken off the screen still lands in the local report log,
// with a typed code and closed data, never text. «Ningún fallo silencioso».

const TOKEN = /^[a-z0-9_.-]{1,64}$/;

function fakePort(overrides: Partial<DaemonPort> = {}) {
  const report = vi.fn<DaemonPort["report"]>(() => Promise.resolve());
  const port: DaemonPort = {
    snapshot: () => Promise.reject(new Error("El daemon dijo algo privado")),
    subscribeFrames: () => Promise.resolve(() => undefined),
    send: () => Promise.resolve({} as never),
    report,
    ...overrides,
  };
  return { port, report };
}

function expectClosed(line: Report) {
  expect(line.code).toMatch(TOKEN);
  for (const value of Object.values(line.data)) {
    if (typeof value === "string") expect(value).toMatch(TOKEN);
  }
  expect(JSON.stringify(line)).not.toMatch(/privado|daemon dijo|ASR|HTTP 5|bytes/);
}

/** A composed view with only what each case sets; the rest is absent. */
function view(parts: Record<string, unknown>): AppView {
  return {
    calendar: { reading: { state: "available" }, writes: [] },
    settings: { update: { state: "idle" } },
    ...parts,
  } as unknown as AppView;
}

function only(reports: ReturnType<typeof stateFailures>) {
  expect(reports).toHaveLength(1);
  return reports[0]!;
}

describe("the connection that failed is reported", () => {
  it("without a refusal: connect_failed, from the interface, no text", async () => {
    const { port, report } = fakePort();
    await createOrbitStore(port).getState().connect();
    expect(report).toHaveBeenCalledTimes(1);
    const line = report.mock.calls[0]![0];
    expect(line).toMatchObject({ code: "connect_failed", class: "silent", source: "ui", data: {} });
    expectClosed(line);
  });

  it("with a refusal: the refusal's own code", async () => {
    const { port, report } = fakePort({
      snapshot: () => Promise.reject({ refusal: { code: "timeout", retry: "never" } }),
    });
    await createOrbitStore(port).getState().connect();
    expect(report.mock.calls.map(([line]) => [line.code, line.source])).toEqual([
      ["timeout", "app"],
    ]);
  });
});

describe("each failure the screen no longer explains is reported", () => {
  it("a note read that failed", () => {
    const failure = only(
      stateFailures(
        view({
          notes: {
            open_read: { recording_id: "rec-1", revision: 3, state: "failed", detail: "privado" },
          },
        }),
      ),
    );
    expect(failure).toMatchObject({ code: "note_read_problem", data: { stage: "failed" } });
  });

  it("a note read with parts missing", () => {
    const failure = only(
      stateFailures(
        view({
          notes: {
            open_read: {
              recording_id: "rec-1",
              revision: 3,
              state: "available",
              detail: "Transcripción: privado",
            },
          },
        }),
      ),
    );
    expect(failure).toMatchObject({ code: "note_read_problem", data: { stage: "incomplete" } });
  });

  it("a transcript that could not be read", () => {
    const failure = only(
      stateFailures(
        view({
          notes: {
            open: { recording_id: "rec-1", transcript: { error: "El archivo ASR no supera" } },
          },
        }),
      ),
    );
    expect(failure.code).toBe("transcript_read_failed");
  });

  // A run of the real test on the Mac found that, before pairing, the hub out
  // of reach was logged as two failures. Without the hub, notes and templates
  // are unknown, not failed.
  it("nothing when the notes and the templates need the hub", () => {
    expect(
      stateFailures(
        view({
          notes: {
            index: { state: "needs_hub", more: false, detail: null },
            library: {
              read: "available",
              templates: { state: "needs_hub", detail: null },
              links: { state: "needs_hub", detail: null },
              items: [],
              assignments: [],
              refusals: [],
              restores: [],
            },
          },
        }),
      ),
    ).toEqual([]);
  });

  it("a notes index that failed", () => {
    const failure = only(
      stateFailures(view({ notes: { index: { state: "failed", detail: "privado" } } })),
    );
    expect(failure.code).toBe("notes_index_failed");
  });

  it("while Orbit closes, the reads it cuts off are not failures", () => {
    const failing = view({ notes: { index: { state: "failed", detail: "cerrado" } } });
    expect(stateFailures(failing, true)).toEqual([]);
    expect(stateFailures(failing)).toHaveLength(1);
  });

  it("an update that failed, with its cause kind as a token", () => {
    const failure = only(
      stateFailures(
        view({
          settings: {
            update: {
              state: "failed",
              refusal: { code: "update", cause: { kind: "http_status", url: "https://x" } },
            },
          },
        }),
      ),
    );
    expect(failure).toMatchObject({
      code: "update_failed",
      data: { stage: "check", cause: "http_status" },
    });
  });

  it("an installation that did not finish", () => {
    const failure = only(
      stateFailures(
        view({
          settings: {
            update: {
              state: "idle",
              last_installation: { failure: "install.sh terminó con estado 1" },
            },
          },
        }),
      ),
    );
    expect(failure).toMatchObject({ code: "update_failed", data: { stage: "install" } });
  });

  it("a template library that could not be read", () => {
    const failure = only(
      stateFailures(
        view({
          notes: {
            library: {
              read: "failed",
              read_detail: "privado",
              templates: { state: "failed" },
              refusals: [],
            },
          },
        }),
      ),
    );
    expect(failure.code).toBe("library_read_failed");
  });

  it("a change the hub refused, with the hub's code and no message", () => {
    const failure = only(
      stateFailures(
        view({
          notes: {
            library: {
              read: "available",
              templates: { state: "available" },
              refusals: [
                {
                  target_kind: "template",
                  target_id: "t-1",
                  action: "save",
                  status: 409,
                  code: "note_templates_full",
                },
              ],
            },
          },
        }),
      ),
    );
    expect(failure).toMatchObject({
      code: "library_refused",
      data: { code: "note_templates_full" },
    });
  });

  it("an agenda read that failed", () => {
    const failure = only(
      stateFailures(
        view({
          calendar: {
            reading: { state: "unavailable", detail: "agenda_request_failed:privado" },
            writes: [],
          },
        }),
      ),
    );
    expect(failure.code).toBe("agenda_read_failed");
  });

  it("a time zone the system could not give, with its cause as a token", () => {
    const failure = only(
      stateFailures(view({ system_timezone_error: "timezone_io_failed:privado" })),
    );
    expect(failure).toMatchObject({
      code: "timezone_failed",
      data: { cause: "timezone_io_failed" },
    });
  });

  it("a calendar write refused or of unknown outcome, with its code and stage", () => {
    const reports = stateFailures(
      view({
        calendar: {
          reading: { state: "available" },
          writes: [
            { request_id: "w-1", stage: "refused", code: "not_editable", notice: "privado" },
            { request_id: "w-2", stage: "outcome_unknown", notice: "privado" },
            { request_id: "w-3", stage: "saved" },
          ],
        },
      }),
    );
    expect(reports.map((report) => [report.code, report.data])).toEqual([
      ["calendar_write_failed", { code: "not_editable", stage: "refused" }],
      ["calendar_write_failed", { stage: "outcome_unknown" }],
    ]);
  });

  it("no data value is free text", () => {
    const reports = stateFailures(
      view({
        system_timezone_error: "El sistema dijo algo privado",
        notes: { open: { recording_id: "rec-1", review_detail: "privado" } },
      }),
    );
    for (const report of reports)
      expectClosed({ op: "0123abcd", class: "silent", source: "ui", ...report });
  });
});

describe("a failure is reported when it appears, once", () => {
  it("not again while it stays, again when it comes back", () => {
    const watch = watchFailures();
    const failing = view({ notes: { index: { state: "failed" } } });
    const fine = view({ notes: { index: { state: "available" } } });
    expect(watch(failing).map((report) => report.code)).toEqual(["notes_index_failed"]);
    expect(watch(failing)).toEqual([]);
    expect(watch(fine)).toEqual([]);
    expect(watch(failing).map((report) => report.code)).toEqual(["notes_index_failed"]);
  });

  it("the store reports a failure that arrives in a frame", async () => {
    const { port, report } = fakePort();
    let listener: ((frame: FeatureFrame) => void) | undefined;
    port.subscribeFrames = (next) => {
      listener = next;
      return Promise.resolve(() => undefined);
    };
    port.snapshot = () => Promise.resolve(bootstrap());
    const store = createOrbitStore(port);
    await store.getState().connect();
    expect(report).not.toHaveBeenCalled();
    listener?.(frame("notes", { list: [], index: { state: "failed", total: 0, loaded: 0 } }, 2));
    expect(report.mock.calls.map(([line]) => [line.code, line.class, line.source])).toEqual([
      ["notes_index_failed", "silent", "ui"],
    ]);
  });

  // Real runs of 2026-10-20: 0.55–0.8 s after `orbit_closing` the list was read
  // again, the read failed and a normal quit counted as a failure. The capture's
  // confirmation follows the closing notice 3 ms later, so it is no longer the
  // latest one when the read fails.
  const closing = {
    code: "orbit_closing",
    op: "op-close",
    sequence: 7,
    data: { cause: "shutdown" },
  };
  const confirmed = { code: "capture_cut_confirmed", op: "op-close", sequence: 8, data: {} };
  const failedIndex = { list: [], index: { state: "failed", total: 0, loaded: 0 } };

  it("once the daemon says it is closing, a read it cuts off is not reported", async () => {
    const { port, report } = fakePort();
    let listener: ((frame: FeatureFrame) => void) | undefined;
    port.subscribeFrames = (next) => {
      listener = next;
      return Promise.resolve(() => undefined);
    };
    port.snapshot = () => Promise.resolve(bootstrap());
    const store = createOrbitStore(port);
    await store.getState().connect();
    const status = { connection: { daemon: "ready", hub: "paired" }, settings: {} };
    listener?.(frame("status", { ...status, notices: [closing], notice: closing }, 2));
    listener?.(frame("status", { ...status, notices: [closing, confirmed], notice: confirmed }, 3));
    listener?.(frame("notes", failedIndex, 2));
    expect(report.mock.calls.map(([line]) => line.code)).not.toContain("notes_index_failed");
  });

  it("the last session's closing notice does not hide this session's failures", async () => {
    const { port, report } = fakePort();
    let listener: ((frame: FeatureFrame) => void) | undefined;
    port.subscribeFrames = (next) => {
      listener = next;
      return Promise.resolve(() => undefined);
    };
    // The notice log is durable: at the next start its newest notice is the
    // closing of the session before.
    const started = bootstrap() as { frames: FeatureFrame[] };
    for (const item of started.frames)
      if (item.feature === "status")
        Object.assign(item.view, { notices: [closing], notice: closing });
    port.snapshot = () => Promise.resolve(started as never);
    const store = createOrbitStore(port);
    await store.getState().connect();
    listener?.(frame("notes", failedIndex, 2));
    expect(report.mock.calls.map(([line]) => line.code)).toContain("notes_index_failed");
  });
});

describe("reads and the panel's links report their refusals", () => {
  it("a read the daemon refused is reported with the refusal's code", async () => {
    const { port, report } = fakePort({
      readSessionsPage: () =>
        Promise.resolve({ state: "absent", refusal: { code: "hub_unreachable", retry: "never" } }),
      readSessionMessages: () => Promise.reject(new Error("privado")),
    });
    const reads = reportingReads(port);
    await reads.readSessionsPage?.(null);
    await expect(reads.readSessionMessages?.("session", null)).rejects.toThrow();
    expect(report.mock.calls.map(([line]) => [line.code, line.source])).toEqual([
      ["hub_unreachable", "app"],
      ["internal", "app"],
    ]);
    for (const [line] of report.mock.calls) expectClosed(line);
  });

  it("a link the panel could not open is reported with the refusal's code", async () => {
    const { port, report } = fakePort({
      openUrl: () =>
        Promise.reject({
          refusal: { code: "invalid_parameter", parameter: "url", retry: "never" },
        }),
    });
    expect(await openPanelLink(port, "javascript:alert(1)")).toBe(es.meetingQuestion.linkRefused);
    expect(report.mock.calls.map(([line]) => [line.code, line.class])).toEqual([
      ["invalid_parameter", "failed"],
    ]);
  });
});

const FEATURES = [
  "status",
  "permissions",
  "agenda",
  "notes",
  "note",
  "recording",
  "assistant",
  "control",
  "briefing",
  "local",
] as const;

function frame(feature: string, view: Record<string, unknown>, counter: number): FeatureFrame {
  return { feature, view, revision: { epoch: 1, counter } } as unknown as FeatureFrame;
}

function bootstrap() {
  const views: Record<string, Record<string, unknown>> = {
    status: { connection: { daemon: "ready", hub: "paired" }, settings: {}, notices: [] },
    agenda: { reading: { state: "available" }, writes: [] },
    notes: { list: [], index: { state: "available", total: 0, loaded: 0 } },
    local: { update: { state: "idle" } },
    recording: { meeting: { phase: "idle" } },
  };
  return {
    revision: { epoch: 1, counter: 1 },
    frames: FEATURES.map((feature) => frame(feature, views[feature] ?? {}, 1)),
  } as never;
}
