import { cleanup, render } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { Icon, iconNames } from "./Icon";

afterEach(cleanup);

it("has the annex's icons beside the redesign's", () => {
  for (const name of ["gear", "copy", "listen", "download", "expand", "conversation", "stop"])
    expect(iconNames).toContain(name);
});

it("draws the annex's icons with its 1.8 stroke, the pencil with its second line", () => {
  const { container } = render(<Icon name="edit" size={16} />);
  const svg = container.querySelector("svg");
  expect(svg?.getAttribute("stroke-width")).toBe("1.8");
  expect(svg?.querySelectorAll("path")).toHaveLength(2);
});

it("draws at 15 points, the size of the icons under an answer", () => {
  const { container } = render(<Icon name="copy" size={15} />);
  expect(container.querySelector("svg")?.getAttribute("width")).toBe("15");
});

it("draws «Reuniones» and a note as the brand's icons", () => {
  const { container } = render(
    <>
      <Icon name="meetings" />
      <Icon name="notes" />
    </>,
  );
  const [meetings, notes] = Array.from(container.querySelectorAll("svg"));
  expect(meetings?.getAttribute("stroke-width")).toBe("1.7");
  expect(Array.from(meetings?.children ?? []).map((path) => path.getAttribute("d"))).toEqual([
    "M4 10.5v3M8 7v10M12 4.5v15M16 7v10M20 10.5v3",
  ]);
  expect(notes?.getAttribute("stroke-width")).toBe("1.7");
  expect(
    Array.from(notes?.children ?? []).map((shape) =>
      shape.tagName === "path" ? shape.getAttribute("d") : shape.getAttribute("r"),
    ),
  ).toEqual(["M5 4.5h9.5L19 9v10.5H5z", "M14.5 4.5V9H19", "M8.5 13.5h3.5M8.5 16.5h7", "1.1"]);
});

it("has no planet with a ring: the brand forbids it", () => {
  expect(iconNames).not.toContain("logo");
});
