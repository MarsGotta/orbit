import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { CopyProvider } from "../i18n/CopyProvider";
import { recordingMeeting } from "../showcase/data/recording";
import { RecordingCard } from "./RecordingCard";
import { RecordingCard as PanelRecordingCard } from "../features/call-panel/CallPanel";

const languages = [
  {
    language: "en",
    action: "End without saving",
    question: "End without saving?",
    keep: "Keep recording",
    detail: "The recording is deleted and no note is written.",
    mute: "Mute",
    help: "Leaves your voice out of Orbit’s recording. Your video call is unchanged.",
    muted: "Your voice is left out of Orbit’s recording. Your video call is unchanged.",
  },
  {
    language: "es",
    action: "Terminar sin guardar",
    question: "¿Terminar sin guardar?",
    keep: "Seguir grabando",
    detail: "Se borra lo grabado y no se escribe nota.",
    mute: "Silenciar",
    help: "Deja tu voz fuera de la grabación de Orbit. Tu videollamada sigue igual.",
    muted: "Tu voz queda fuera de la grabación de Orbit. Tu videollamada sigue igual.",
  },
] as const;

for (const copy of languages) {
  for (const panel of [false, true]) {
    function show(muted = false) {
      const send = vi.fn(async () => true);
      const meeting = { ...recordingMeeting, microphone_muted: muted };
      const view = render(
        <CopyProvider choice={copy.language}>
          {panel ? (
            <PanelRecordingCard call={null} meeting={meeting} send={send} voice={null} />
          ) : (
            <RecordingCard meeting={meeting} send={send} timezone="UTC" />
          )}
        </CopyProvider>,
      );
      return { send, ...view };
    }

    it(`${copy.language} ${panel ? "panel" : "sidebar"}: discarding focuses Keep, describes the loss, and restores focus on either cancellation`, async () => {
      const { send } = show();
      for (const escape of [true, false]) {
        await userEvent.click(screen.getByRole("button", { name: copy.action }));
        const question = screen.getByRole("group", { name: copy.question });
        const keep = within(question).getByRole("button", { name: copy.keep });
        expect(document.activeElement).toBe(keep);
        const description = question.getAttribute("aria-describedby");
        expect(description && document.getElementById(description)?.textContent).toBe(copy.detail);
        if (escape) await userEvent.keyboard("{Escape}");
        else await userEvent.click(keep);
        expect(screen.queryByRole("group", { name: copy.question })).toBeNull();
        expect(document.activeElement).toBe(screen.getByRole("button", { name: copy.action }));
        expect(send).not.toHaveBeenCalled();
      }
    });

    it(`${copy.language} ${panel ? "panel" : "sidebar"}: scopes microphone mute to Orbit’s recording`, async () => {
      const { send, unmount } = show();
      const mute = screen.getByRole("button", { name: copy.mute });
      expect(mute.title).toBe(copy.help);
      await userEvent.click(mute);
      expect(send).toHaveBeenCalledWith({ command: "microphone_mute", muted: true });
      unmount();
      show(true);
      expect(screen.getByText(copy.muted)).toBeTruthy();
    });
  }
}
