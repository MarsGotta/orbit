import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { flushBeforeQuit, useFlushOnQuit } from "./quit-flush";

// Before Orbit quits or installs an update, every view hands over what it
// still holds, and the quit goes on once all of it has been handed over.

function Saving({ save }: { save: () => unknown }) {
  useFlushOnQuit(save);
  return null;
}

afterEach(cleanup);

describe("the quit flush", () => {
  it("asks every view on screen for what it still holds, and ends only when all have answered", async () => {
    let finish: () => void = () => undefined;
    const slow = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
    const quick = vi.fn(() => undefined);
    render(
      <>
        <Saving save={slow} />
        <Saving save={quick} />
      </>,
    );
    let done = false;
    const flushing = flushBeforeQuit().then(() => {
      done = true;
    });
    await Promise.resolve();
    expect(slow).toHaveBeenCalledTimes(1);
    expect(quick).toHaveBeenCalledTimes(1);
    expect(done).toBe(false);
    finish();
    await flushing;
    expect(done).toBe(true);
  });

  it("a view that is gone is not asked", async () => {
    const save = vi.fn();
    const view = render(<Saving save={save} />);
    view.unmount();
    await flushBeforeQuit();
    expect(save).not.toHaveBeenCalled();
  });

  it("asks with the view's latest save", async () => {
    const first = vi.fn();
    const latest = vi.fn();
    const view = render(<Saving save={first} />);
    view.rerender(<Saving save={latest} />);
    await flushBeforeQuit();
    expect(first).not.toHaveBeenCalled();
    expect(latest).toHaveBeenCalledTimes(1);
  });

  it("a save that fails holds neither the others nor the quit", async () => {
    const other = vi.fn();
    render(
      <>
        <Saving
          save={() => {
            throw new Error("roto");
          }}
        />
        <Saving save={() => Promise.reject(new Error("rechazado"))} />
        <Saving save={other} />
      </>,
    );
    await expect(flushBeforeQuit()).resolves.toBeUndefined();
    expect(other).toHaveBeenCalledTimes(1);
  });
});
