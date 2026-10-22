import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TurnText } from "./TurnText";
import { offsetAtPoint } from "./word-at-point";

// Where the pointer is, the browser says from its layout (caretRangeFromPoint),
// which the test DOM has none of: the test says which character it is on.
vi.mock("./word-at-point", async (original) => ({
  ...(await original<typeof import("./word-at-point")>()),
  offsetAtPoint: vi.fn(() => null),
}));

afterEach(() => {
  cleanup();
  vi.mocked(offsetAtPoint).mockReset();
});

const text = "Ya está en Orvit. Lo probamos";

function turn(onPick = vi.fn()) {
  render(
    <TurnText
      anchorRef={() => undefined}
      fixed={[]}
      focused={undefined}
      matches={[]}
      onPick={onPick}
      picked={null}
      text={text}
    />,
  );
  return { paragraph: screen.getByText(/^Ya está en/), onPick };
}

describe("a word of the transcript under the pointer", () => {
  it("lights up whole, with «Pulsa para corregir», and nothing else of the turn changes", () => {
    const { paragraph } = turn();
    vi.mocked(offsetAtPoint).mockReturnValue(13);
    fireEvent.mouseMove(paragraph, { clientX: 1, clientY: 1 });
    const lit = paragraph.querySelector("[data-hover]");
    expect(lit?.firstChild?.textContent).toBe("Orvit");
    expect(lit?.textContent).toBe("OrvitPulsa para corregir");
    expect(lit?.querySelector("[aria-hidden='true']")?.textContent).toBe("Pulsa para corregir");
    expect(paragraph.textContent?.replace("Pulsa para corregir", "")).toBe(text);
  });

  it("goes dark on a space, a sign or when the pointer leaves", () => {
    const { paragraph } = turn();
    vi.mocked(offsetAtPoint).mockReturnValue(13);
    fireEvent.mouseMove(paragraph);
    vi.mocked(offsetAtPoint).mockReturnValue(16);
    fireEvent.mouseMove(paragraph);
    expect(paragraph.querySelector("[data-hover]")).toBeNull();
    vi.mocked(offsetAtPoint).mockReturnValue(13);
    fireEvent.mouseMove(paragraph);
    fireEvent.mouseLeave(paragraph);
    expect(paragraph.querySelector("[data-hover]")).toBeNull();
  });

  it("does not light up while the mouse selects text", () => {
    const { paragraph } = turn();
    vi.mocked(offsetAtPoint).mockReturnValue(13);
    fireEvent.mouseMove(paragraph, { buttons: 1 });
    expect(paragraph.querySelector("[data-hover]")).toBeNull();
  });

  it("is the word a press opens, where it sits in the turn", () => {
    const { paragraph, onPick } = turn();
    vi.mocked(offsetAtPoint).mockReturnValue(4);
    fireEvent.mouseMove(paragraph);
    fireEvent.click(paragraph);
    expect(onPick).toHaveBeenCalledExactlyOnceWith({ start: 3, end: 7 });
  });
});
