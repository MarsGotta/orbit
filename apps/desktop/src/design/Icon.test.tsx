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
