import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { MeetingSnapshot } from "../../bridge/generated/MeetingSnapshot";
import { AskMenu } from "./AskMenu";

it("the sidebar context choices use one tab stop and wrap selection with arrows", async () => {
  const sent: AppCommand[] = [];
  render(
    <AskMenu
      meeting={
        {
          phase: "recording",
          ask: {
            windows: [{ minutes: 2 }, { minutes: 5 }, { minutes: 10 }],
            default_minutes: 5,
            max_question_bytes: 512,
          },
        } as MeetingSnapshot
      }
      send={(command) => sent.push(command)}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Preguntar a Deimos" }));
  const five = screen.getByRole("radio", { name: "5 min" });
  expect(five.tabIndex).toBe(0);
  five.focus();
  await userEvent.keyboard("{ArrowLeft}{ArrowUp}");
  const ten = screen.getByRole("radio", { name: "10 min" });
  expect(ten).toBe(document.activeElement);
  expect(ten.getAttribute("aria-checked")).toBe("true");
  expect(five.tabIndex).toBe(-1);
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "What changed?" } });
  await userEvent.click(screen.getByRole("button", { name: "Preguntar" }));
  expect(sent).toEqual([{ command: "meeting_ask", question: "What changed?", minutes: 10 }]);
});
