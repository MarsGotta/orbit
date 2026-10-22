import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { MeetingAudio } from "../../bridge/generated/MeetingAudio";
import type { AppView } from "../../state/app-view";
import { MeetingStartDialog } from "./MeetingStartDialog";

afterEach(cleanup);

const sources: [string, MeetingAudio][] = [
  ["system", "system"],
  ["microphone", "microphone_only"],
  ["application:4242", { application: { application_pid: 4242 } }],
];

it.each(sources)("recording with %s sends orbit-core's audio selection", (source, audio) => {
  const send = vi.fn();
  const close = vi.fn();
  const snapshot = {
    privacy_stopped: false,
    permissions: { microphone: "granted", screen: "granted" },
    meeting: {
      applications: [{ name: "Orbit Pruebas", pid: 4242 }],
      applications_state: "ready",
    },
  } as AppView;
  render(<MeetingStartDialog onClose={close} send={send} snapshot={snapshot} />);
  expect(send).toHaveBeenCalledWith({ command: "audio_sources_refresh" });
  fireEvent.change(screen.getByRole("combobox"), { target: { value: source } });
  fireEvent.click(screen.getByRole("button", { name: "Empezar a grabar" }));
  expect(send).toHaveBeenLastCalledWith({
    command: "meeting_start",
    title: "",
    audio,
  });
  expect(close).toHaveBeenCalledOnce();
});
