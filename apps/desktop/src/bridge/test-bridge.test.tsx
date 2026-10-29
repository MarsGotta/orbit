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
      expect.arrayContaining([
        expect.objectContaining({ role: "button", name: "Record", text: "Record" }),
      ]),
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
  it("preserves a semantic tree and can address one repeated button without relaxing name guards", () => {
    const clicked: string[] = [];
    render(
      <>
        <h2>Meetings</h2>
        <button aria-label="Refresh" />
        <section>
          <h3>Today</h3>
          <a href="#meeting">
            <span>Invented meeting</span>
            <time>10:00</time>
          </a>
        </section>
        <button onClick={() => clicked.push("first")}>Repeated</button>
        <button onClick={() => clicked.push("second")}>Repeated</button>
      </>,
    );
    const read = request("read");
    expect(read.tree_markdown).toContain('AXStaticText = "Meetings"');
    const rows = read.elements as { role: string; name: string; target: number[] }[];
    const repeats = rows.filter((row) => row.role === "button" && row.name === "Repeated");
    expect(repeats).toHaveLength(2);
    act(() => {
      expect(
        evaluate({
          command: "press",
          window: "main",
          role: "button",
          name: "Repeated",
          target: repeats[1]!.target,
          deadline: Date.now() + 2000,
        }),
      ).toMatchObject({ ok: true });
    });
    expect(clicked).toEqual(["second"]);
    expect(request("press", "Repeated")).toMatchObject({ ok: false, count: 2 });
    expect(
      evaluate({
        command: "press",
        role: "button",
        name: "Changed",
        target: repeats[1]!.target,
        deadline: Date.now() + 2000,
      }),
    ).toMatchObject({ ok: false, count: 0 });
  });
  it("refuses a target after the DOM node was replaced, even with the same name", () => {
    const { rerender } = render(<button key="before">Save</button>);
    const row = (
      request("read").elements as { role: string; name: string; target: number[] }[]
    ).find((row) => row.role === "button")!;
    const button = screen.getByRole("button", { name: "Save" });
    rerender(<button key="after">Save</button>);
    expect(button.isConnected).toBe(false);
    expect(
      evaluate({
        command: "press",
        role: "button",
        name: "Save",
        target: row.target,
        deadline: Date.now() + 2000,
      }),
    ).toMatchObject({ ok: false, cause: "match_count", count: 0 });
  });
});
