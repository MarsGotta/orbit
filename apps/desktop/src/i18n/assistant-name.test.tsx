import { act, render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import type { AppBootstrap, FeatureFrame } from "../bridge/generated";
import type { DaemonPort, FrameListener } from "../bridge/port";
import { spanish } from "./es";
import { receive } from "../app/interceptor";
import { PanelApp } from "../features/call-panel/PanelApp";

it("uses the hub name in the panel and wake status as frames arrive", async () => {
  let listener: FrameListener | undefined;
  let counter = 0;
  const status = (assistant_name?: string) =>
    ({
      feature: "status",
      revision: { epoch: 1, counter: ++counter },
      view: { settings: { assistant_name }, notices: [], notice: null },
    }) as unknown as FeatureFrame;
  const port = {
    subscribeFrames: async (accept: FrameListener) => {
      listener = accept;
      return () => undefined;
    },
    snapshot: async () =>
      ({
        revision: { epoch: 1, counter: 1 },
        frames: [
          status(),
          ...[
            "local",
            "permissions",
            "agenda",
            "notes",
            "note",
            "recording",
            "assistant",
            "control",
            "briefing",
          ].map((feature) => ({
            feature,
            revision: { epoch: 1, counter: 1 },
            view:
              feature === "recording"
                ? {
                    meeting: {
                      question: {
                        id: "q",
                        question: "¿Qué decidió Ana Pérez?",
                        stage: "thinking",
                        sent: true,
                      },
                      voice_trigger: { state: "listening" },
                      ask: {
                        windows: [{ minutes: 2 }],
                        default_minutes: 2,
                        max_question_bytes: 1024,
                      },
                    },
                  }
                : {},
          })),
        ],
      }) as unknown as AppBootstrap,
    report: async () => undefined,
    send: async () => ({ state: "accepted" }),
  } as unknown as DaemonPort;
  render(<PanelApp port={port} />);
  expect(await screen.findByText("Deimos está pensando…")).toBeTruthy();
  await act(async () => listener?.(status("Atlas")));
  expect(screen.getByText("Atlas está pensando…")).toBeTruthy();
  expect(screen.getByText("«Oye Atlas» te escucha")).toBeTruthy();
  expect(screen.getByRole("region", { name: "Atlas en la reunión" })).toBeTruthy();
  expect(screen.getByText("¿Qué decidió Ana Pérez?", { exact: false })).toBeTruthy();
  await act(async () => listener?.(status()));
  expect(screen.getByText("Atlas está pensando…")).toBeTruthy();
});

it("uses the configured name in chat, notes, permissions and notices, preserving supplied text", () => {
  const copy = spanish("Atlas");
  const meeting = copy.meetingQuestion;
  const named = [
    copy.chat.roles.assistant,
    copy.assistant.thinking,
    copy.assistant.responding,
    copy.assistant.noHermesVoice,
    copy.noteScreen.askDeimos,
    copy.shell.ask,
    copy.design.callAssistant,
    copy.design.inCall,
    copy.assistant.pairHub,
    copy.chatActions.sendBack,
    copy.settings.privacyArmed,
    copy.settings.engine.missing("Accesibilidad"),
    copy.settings.engine.failed,
    copy.settings.pairAgainWarning,
    copy.fullControl.noHistory,
    meeting.ask,
    meeting.fieldLabel,
    meeting.panelLabel,
    meeting.thinking,
    meeting.responding,
    meeting.alsoInConversation,
    meeting.failed,
    meeting.notSent,
    meeting.busy.assistant_turn,
    meeting.busy.pending_approval,
    meeting.busy.call,
    meeting.voice.listening,
    meeting.voice.downloading(37),
    meeting.voice.off("en pausa"),
    meeting.voice.short.listening,
    meeting.voice.short.downloading(37),
    meeting.voice.short.off,
  ];
  for (const text of named) {
    expect(text).toContain("Atlas");
    expect(text).not.toContain("Deimos");
  }
  expect(meeting.asked("¿Qué dijo Deimos?")).toBe("Tu pregunta: ¿Qué dijo Deimos?");
  expect(meeting.voice.off("Deimos es el nombre anterior")).toContain(
    "Deimos es el nombre anterior",
  );
  const codes = [
    "assistant_turn_lost_by_disconnect",
    "assistant_outcome_unknown",
    "assistant_interrupt_refused",
    "approval_expired",
    "meeting_question_still_answering",
    "bridge_serving",
  ] as const;
  for (const code of codes) {
    const result = receive(
      [],
      {
        kind: "notice",
        notice: {
          code,
          op: "assistant-name-notice",
          data: { code: "calendar_read" },
        },
      },
      {},
      "Atlas",
    );
    expect(result.messages[0]?.text).toContain("Atlas");
    expect(result.messages[0]?.text).not.toContain("Deimos");
  }
});
