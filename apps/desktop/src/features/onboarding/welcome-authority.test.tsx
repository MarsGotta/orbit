import { render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { CopyProvider } from "../../i18n/CopyProvider";
import type { AppView } from "../../state/app-view";
import { OnboardingView } from "./OnboardingView";

it.each([
  [
    "en",
    "On your Mac, assistants act with the macOS access you give Orbit. On the hub, they ask with a card before each command or edit. Stopping everything cuts them off at once.",
  ],
  [
    "es",
    "En tu Mac, los asistentes actúan con el acceso de macOS que le das a Orbit. En el hub, te piden una tarjeta antes de cada orden o edición. Pararlo todo les corta el acceso al momento.",
  ],
] as const)(
  "%s: the welcome distinguishes Mac access from hub approval",
  (language, authority) => {
    render(
      <CopyProvider choice={language}>
        <OnboardingView
          error={null}
          snapshot={{ settings: { hub: { paired: false }, permissions: {} } } as unknown as AppView}
          send={vi.fn()}
          onComplete={vi.fn()}
        />
      </CopyProvider>,
    );
    expect(screen.getByText(authority)).toBeTruthy();
  },
);
