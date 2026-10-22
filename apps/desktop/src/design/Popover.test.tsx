import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, it } from "vitest";
import { Popover } from "./Popover";

afterEach(cleanup);

function SpeakerMenu() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <p>Fuera del menú</p>
      <Popover
        label="Cambiar quién habla"
        onOpenChange={setOpen}
        open={open}
        trigger="Marta Ibáñez"
      >
        <button type="button">Diego Salinas</button>
      </Popover>
    </>
  );
}

it("opens anchored to its control and puts the focus inside", () => {
  render(<SpeakerMenu />);
  const trigger = screen.getByRole("button", { name: "Marta Ibáñez" });
  expect(trigger.getAttribute("aria-expanded")).toBe("false");
  fireEvent.click(trigger);
  expect(trigger.getAttribute("aria-expanded")).toBe("true");
  expect(screen.getByRole("dialog", { name: "Cambiar quién habla" })).toBeTruthy();
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "Diego Salinas" }));
});

it("Esc closes it and the focus goes back to its control", () => {
  render(<SpeakerMenu />);
  const trigger = screen.getByRole("button", { name: "Marta Ibáñez" });
  fireEvent.click(trigger);
  fireEvent.keyDown(screen.getByRole("button", { name: "Diego Salinas" }), { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(trigger);
});

it("a press outside closes it; one inside does not", () => {
  render(<SpeakerMenu />);
  fireEvent.click(screen.getByRole("button", { name: "Marta Ibáñez" }));
  fireEvent.mouseDown(screen.getByRole("button", { name: "Diego Salinas" }));
  expect(screen.getByRole("dialog")).toBeTruthy();
  fireEvent.mouseDown(screen.getByText("Fuera del menú"));
  expect(screen.queryByRole("dialog")).toBeNull();
});
