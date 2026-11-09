import { cleanup, render } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { AssistantAvatar, type AssistantAvatarSize } from "./AssistantAvatar";

afterEach(cleanup);

it("is the assistant's initial, hidden from the reader: its name is written beside it", () => {
  const { container } = render(<AssistantAvatar color={2} name="phobos" size={28} />);
  const avatar = container.firstElementChild as HTMLElement;
  expect(avatar.textContent).toBe("P");
  expect(avatar.getAttribute("aria-hidden")).toBe("true");
});

it("takes the first letter of a name that starts with an accent or a symbol of two units", () => {
  const { container } = render(
    <>
      <AssistantAvatar color={1} name="Ío" size={18} />
      <AssistantAvatar color={1} name="𝒜res" size={18} />
    </>,
  );
  expect([...container.children].map((avatar) => avatar.textContent)).toEqual(["Í", "𝒜"]);
});

it("comes in the seven sizes the screens use, in its colour's tint", () => {
  const sizes: AssistantAvatarSize[] = [18, 20, 26, 28, 36, 40, 56];
  const { container } = render(
    <>
      {sizes.map((size) => (
        <AssistantAvatar color={3} key={size} name="Ares" size={size} />
      ))}
    </>,
  );
  const avatars = [...container.children] as HTMLElement[];
  expect(avatars.map((avatar) => getComputedStyle(avatar).width)).toEqual(
    sizes.map((size) => `${size}px`),
  );
  for (const avatar of avatars) {
    expect(getComputedStyle(avatar).backgroundColor).toBe("var(--o-c3s)");
    expect(getComputedStyle(avatar).color).toBe("var(--o-c3)");
  }
});
