import { describe, expect, it } from "vitest";
import type { Notice } from "../bridge/generated/Notice";
import type { NoticeCode } from "../bridge/generated/NoticeCode";
import { genericFailure, receive, type Message } from "./interceptor";

const OP = "7f3a2c9e-0000-4000-8000-000000000001";
const OTHER = "a1b2c3d4-0000-4000-8000-000000000002";

function notice(code: string, op = OP, data?: Notice["data"]): Notice {
  return { code: code as NoticeCode, op, data };
}

function only(messages: Message[], op = OP): Message | undefined {
  const matching = messages.filter((message) => message.op === op);
  expect(matching.length).toBeLessThanOrEqual(1);
  return matching[0];
}

describe("the interceptor shows each class as the table says", () => {
  it("fixable, working and done are shown with their own text", () => {
    for (const [code, kind] of [
      ["microphone_permission_missing", "fixable"],
      ["voice_transcribing", "working"],
      ["transcript_retry_requested", "done"],
    ] as const) {
      const { messages, report } = receive([], { kind: "notice", notice: notice(code) });
      const message = only(messages);
      expect(message?.class).toBe(kind);
      expect(message?.text).not.toBe(genericFailure(OP));
      expect(message?.text.length).toBeGreaterThan(0);
      expect(report).toBeNull();
    }
  });

  it("a failure the person cannot fix is the generic message with the op's code", () => {
    const { messages, report } = receive([], { kind: "notice", notice: notice("capture_cut") });
    expect(only(messages)?.text).toBe(
      "No se pudo seguir grabando. Vuelve a intentarlo. Si se repite, comparte el código 7F3A2C con quien gestiona tu hub.",
    );
    expect(only(messages)?.class).toBe("failed");
    expect(report).toBeNull();
  });

  it("a silent notice is never shown", () => {
    const { messages } = receive([], { kind: "notice", notice: notice("audio_gap") });
    expect(messages).toEqual([]);
  });

  it("a silent notice ends a working message of its op, never a failure", () => {
    const working = receive([], { kind: "notice", notice: notice("voice_transcribing") }).messages;
    expect(receive(working, { kind: "notice", notice: notice("audio_gap") }).messages).toEqual([]);
    const fixable = receive([], {
      kind: "notice",
      notice: notice("microphone_permission_missing"),
    }).messages;
    expect(
      only(receive(fixable, { kind: "notice", notice: notice("audio_gap") }).messages)?.class,
    ).toBe("fixable");
  });
});

describe("a code the catalogue does not know", () => {
  it("is the generic failure with the op's code, and is reported", () => {
    for (const code of ["unknown", "code_from_the_future"]) {
      const { messages, report } = receive([], {
        kind: "notice",
        notice: notice(code, OP, { cause: "Hola Alina", count: 2 }),
      });
      expect(only(messages)?.text).toBe(genericFailure(OP));
      expect(report).toEqual({
        op: OP,
        code,
        class: "failed",
        source: "ui",
        data: { count: 2 },
      });
    }
  });

  it("without an op still gets one, so its code can be quoted", () => {
    const { messages, report } = receive([], {
      kind: "notice",
      notice: { code: "code_from_the_future" as NoticeCode },
    });
    const message = messages[0];
    expect(message?.op.length).toBeGreaterThanOrEqual(8);
    expect(message?.text).toBe(genericFailure(message?.op ?? ""));
    expect(report?.op).toBe(message?.op);
  });
});

describe("never two visible messages of the same op", () => {
  it("working, then done, of one op leave only the done", () => {
    let messages = receive([], { kind: "notice", notice: notice("voice_transcribing") }).messages;
    messages = receive(messages, {
      kind: "notice",
      notice: notice("transcript_retry_requested"),
    }).messages;
    expect(only(messages)?.class).toBe("done");
    expect(messages).toHaveLength(1);
  });

  it("a command's refusal and a notice of the same op are one message", () => {
    let messages = receive([], {
      kind: "refusal",
      op: OP,
      refusal: { code: "timeout", retry: "never" },
    }).messages;
    messages = receive(messages, { kind: "notice", notice: notice("pairing_failed") }).messages;
    expect(messages).toHaveLength(1);
    expect(only(messages)?.class).toBe("fixable");
  });

  it("messages of different ops coexist", () => {
    let messages = receive([], { kind: "notice", notice: notice("voice_transcribing") }).messages;
    messages = receive(messages, {
      kind: "notice",
      notice: notice("microphone_permission_missing", OTHER),
    }).messages;
    expect(messages).toHaveLength(2);
  });
});

describe("refusals of an order go through the same catalogue", () => {
  it("a permission refusal is fixable, with the way to fix it", () => {
    const { messages, report } = receive([], {
      kind: "refusal",
      op: OP,
      refusal: { code: "permission_not_granted", permission: "microphone", retry: "never" },
    });
    expect(only(messages)?.class).toBe("fixable");
    expect(only(messages)?.action).toBeDefined();
    expect(report).toMatchObject({ op: OP, code: "permission_not_granted", source: "app" });
  });

  it("a refusal the person cannot act on is the generic failure, and is reported", () => {
    const { messages, report } = receive([], {
      kind: "refusal",
      op: OP,
      refusal: {
        code: "storage_failed",
        operation: "storage",
        cause: "disco lleno",
        retry: "never",
      },
    });
    expect(only(messages)?.text).toBe(genericFailure(OP));
    expect(report).toEqual({
      op: OP,
      code: "storage_failed",
      class: "failed",
      source: "app",
      data: {},
    });
  });

  it("a view that failed to paint is the generic failure, reported from the interface", () => {
    const { messages, report } = receive([], {
      kind: "crash",
      op: OP,
      code: "ui_render_failed",
    });
    expect(only(messages)?.text).toBe(genericFailure(OP, "mostrar esta vista"));
    expect(report).toMatchObject({ code: "ui_render_failed", class: "failed", source: "ui" });
  });
});

describe("recovery is work in progress", () => {
  it("shows the recovery step and elapsed time without a corrective action", () => {
    const { messages, report } = receive([], {
      kind: "refusal",
      op: OP,
      refusal: { code: "recovering", step: "snapshot", elapsed_ms: 1234, retry: { kind: "never" } },
    });
    expect(only(messages)?.class).toBe("working");
    expect(only(messages)?.text).toBe(
      "Orbit está recuperando el almacén: paso instantánea, 1234 ms transcurridos. Espera a que termine.",
    );
    expect(only(messages)?.action).toBeUndefined();
    expect(report?.class).toBe("working");
  });
});

describe("repeated notices", () => {
  it("replaces an identical visible notice from a different operation", () => {
    let messages = receive([], {
      kind: "notice",
      notice: notice("pending_work_keep_open"),
    }).messages;
    messages = receive(messages, {
      kind: "notice",
      notice: notice("microphone_permission_missing", "permission-op"),
    }).messages;
    messages = receive(messages, {
      kind: "notice",
      notice: notice("pending_work_keep_open", OTHER),
    }).messages;
    expect(messages).toHaveLength(2);
    expect(
      messages.filter(
        (message) => message.text === "Orbit sigue guardando. No lo cierres todavía.",
      ),
    ).toEqual([
      { op: OTHER, class: "fixable", text: "Orbit sigue guardando. No lo cierres todavía." },
    ]);
    expect(messages.some((message) => message.op === "permission-op")).toBe(true);
  });

  it("keeps notices whose visible data differs", () => {
    const first = receive([], {
      kind: "notice",
      notice: notice("audio_route_recovered", OP, { elapsed_ms: 1000 }),
    }).messages;
    const second = receive(first, {
      kind: "notice",
      notice: notice("audio_route_recovered", OTHER, { elapsed_ms: 2000 }),
    }).messages;
    expect(second).toHaveLength(2);
  });
});
