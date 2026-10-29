import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, beforeEach, vi } from "vitest";
import source from "../../src-tauri/src/test_bridge/dom.js?raw";

function evaluate(payload: object): Record<string, unknown> {
  // Use the same invocation shape as the shell, including the raw source's
  // trailing semicolon. A mock web view cannot catch JavaScript syntax errors.
  return window.eval(
    `(() => { const run = ${source}; return run(${JSON.stringify(payload)}); })()`,
  );
}
function request(command: string, name = "", role = "button", text = "") {
  return evaluate({
    command,
    name,
    role,
    text,
    deadline: Date.now() + 2000,
    max_reply_bytes: 100_000,
  });
}

function Fixture() {
  const [value, setValue] = useState("");
  const [pressed, setPressed] = useState(0);
  return (
    <>
      <button onClick={() => setPressed(pressed + 1)}>Record</button>
      <button>Repeated</button>
      <button>Repeated</button>
      <button hidden>Hidden</button>
      <label>
        Message
        <input value={value} onChange={(e) => setValue(e.target.value)} />
      </label>
      <output>
        {value}:{pressed}
      </output>
    </>
  );
}
describe("test bridge DOM requests", () => {
  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, "getClientRects").mockReturnValue([
      new DOMRect(0, 0, 100, 30),
    ] as unknown as DOMRectList);
  });
  it("clicks the named DOM element and types through React's controlled field", () => {
    render(<Fixture />);
    act(() => {
      expect(request("press", "Record").ok).toBe(true);
    });
    expect(screen.getByRole("status").textContent).toBe(":1");
    act(() => {
      expect(request("type", "Message", "textbox", "Hello").ok).toBe(true);
    });
    expect(screen.getByRole("status").textContent).toBe("Hello:1");
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "Again" } });
    expect(screen.getByRole("status").textContent).toBe("Again:1");
  });
  it("refuses zero or two matches with names and counts", () => {
    render(<Fixture />);
    for (const [name, count] of [
      ["Missing", 0],
      ["Repeated", 2],
      ["Hidden", 0],
    ] as const) {
      expect(request("press", name)).toMatchObject({
        ok: false,
        cause: "match_count",
        name,
        count,
      });
    }
  });
  it("reads visible roles, names and text, and refuses an expired action", () => {
    render(<Fixture />);
    expect(request("read").elements).toEqual(
      expect.arrayContaining([{ role: "button", name: "Record", text: "Record" }]),
    );
    expect(
      evaluate({ command: "press", role: "button", name: "Record", deadline: 0 }),
    ).toMatchObject({ ok: false, cause: "view_deadline_expired" });
    expect(screen.getByRole("status").textContent).toBe(":0");
  });
  it("refuses an oversized DOM reading before it crosses the view callback", () => {
    render(<Fixture />);
    const reply = evaluate({ command: "read", deadline: Date.now() + 2000, max_reply_bytes: 80 });
    expect(reply).toMatchObject({ ok: false, cause: "reply_too_large", max_bytes: 80 });
    expect(Number(reply.bytes)).toBeGreaterThan(80);
  });
  it("uses accessible names without hidden decoration and refuses read-only fields", () => {
    render(
      <>
        <button>
          <span aria-hidden="true">Decoration</span>Save
        </button>
        <input aria-label="Identifier" readOnly value="fixed" />
      </>,
    );
    expect(request("press", "Save")).toMatchObject({ ok: true, count: 1 });
    expect(request("type", "Identifier", "textbox", "changed")).toMatchObject({
      ok: false,
      cause: "field_read_only",
    });
    expect((screen.getByRole("textbox") as HTMLInputElement).value).toBe("fixed");
  });
});
