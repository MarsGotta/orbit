import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { AppCommand } from "../../bridge/generated";
import type { MeetingAskSnapshot } from "../../bridge/generated/MeetingAskSnapshot";
import type { MeetingQuestionCause } from "../../bridge/generated/MeetingQuestionCause";
import type { MeetingQuestionSnapshot } from "../../bridge/generated/MeetingQuestionSnapshot";
import type { VoiceOffCause } from "../../bridge/generated/VoiceOffCause";
import { es } from "../../i18n/es";
import { AskDeimos } from "./AskDeimos";
import { causeText, voiceStatusText } from "./copy";
import { QuestionCard } from "./QuestionCard";

/** What app-core publishes; the interface never writes these numbers. */
const ASK: MeetingAskSnapshot = {
  windows: [{ minutes: 2 }, { minutes: 5 }, { minutes: 10 }],
  default_minutes: 2,
  max_question_bytes: 40,
};

function question(overrides: Partial<MeetingQuestionSnapshot> = {}): MeetingQuestionSnapshot {
  return {
    id: "turn-1",
    question: "¿Qué dijo Lucía del presupuesto?",
    minutes: 2,
    origin: "voice",
    stage: "thinking",
    sent: true,
    ...overrides,
  };
}

function card(snapshot: MeetingQuestionSnapshot) {
  const sent: AppCommand[] = [];
  const view = render(<QuestionCard question={snapshot} send={(command) => sent.push(command)} />);
  const region = screen.getByRole("region", { name: es.meetingQuestion.panelLabel });
  return { region, sent, view };
}

describe("P9-12 the answer panel", () => {
  it("says Deimos is thinking, with the question, and shows the answer as it arrives", async () => {
    const thinking = card(question());
    expect(thinking.region.textContent).toContain(es.meetingQuestion.thinking);
    expect(thinking.region.textContent).toContain("¿Qué dijo Lucía del presupuesto?");
    expect(thinking.region.getAttribute("aria-busy")).toBe("true");
    thinking.view.unmount();

    const partial = card(question({ stage: "responding", answer: "Que el presupuesto" }));
    expect(partial.region.textContent).toContain(es.meetingQuestion.responding);
    expect(partial.region.textContent).toContain("Que el presupuesto");
    partial.view.unmount();

    const answered = card(
      question({ stage: "answered", answer: "Que sube un 10 %.", shown_until_ms: 45 }),
    );
    expect(answered.region.textContent).toContain("Que sube un 10 %.");
    expect(answered.region.textContent).not.toContain(es.meetingQuestion.thinking);
    expect(answered.region.textContent).toContain(es.meetingQuestion.alsoInConversation);
    await userEvent.click(screen.getByRole("button", { name: es.meetingQuestion.close }));
    expect(answered.sent).toEqual([{ command: "meeting_question_close", id: "turn-1" }]);
  });

  // SIM-24: each failure says what to do, in one line; the codes and the sizes
  // stay in the log, never on screen.
  it("shows each failure as what to do, without codes or numbers", () => {
    const causes = es.meetingQuestion.causes;
    const cases: [MeetingQuestionCause, string][] = [
      [{ cause: "privacy_stopped" }, causes.privacyStopped],
      [{ cause: "no_meeting" }, causes.noMeeting],
      [{ cause: "hub_unreachable", connection: "offline" }, causes.hubUnreachable],
      [{ cause: "deimos_unavailable", code: "upstream_unavailable" }, causes.tryAgain],
      [{ cause: "turn_in_flight", turn: "assistant_turn" }, es.meetingQuestion.busy.assistant_turn],
      [{ cause: "question_too_long", bytes: 1100, max_bytes: 1024 }, causes.questionTooLong],
      [{ cause: "window_not_offered", minutes: 7, offered: [2, 5, 10] }, causes.windowNotOffered],
      [{ cause: "turn_failed" }, causes.tryAgain],
      [{ cause: "interrupted" }, causes.interrupted],
      [{ cause: "hub_refused", code: "send_failed", message: "canal cerrado" }, causes.tryAgain],
      [{ cause: "no_answer" }, causes.tryAgain],
      [{ cause: "revoked" }, causes.revoked],
      [{ cause: "question_empty" }, causes.questionEmpty],
    ];
    for (const [cause, text] of cases) {
      expect(causeText(cause)).toBe(text);
      const failed = card(question({ stage: "failed", cause, shown_until_ms: 8 }));
      expect(screen.getByRole("alert").textContent).toBe(text);
      expect(failed.region.textContent).not.toMatch(
        /upstream_unavailable|send_failed|canal cerrado|1100|1024|undefined/,
      );
      failed.view.unmount();
    }
  });
});

function ask(props: Partial<Parameters<typeof AskDeimos>[0]> = {}) {
  const sent: AppCommand[] = [];
  let done = 0;
  render(
    <AskDeimos
      ask={ASK}
      onDone={() => (done += 1)}
      send={(command) => sent.push(command)}
      {...props}
    />,
  );
  return { sent, done: () => done };
}

describe("P9-12 «Preguntar a Deimos»", () => {
  it("asks with the windows app-core offers, two minutes by default", async () => {
    const { sent, done } = ask();
    const windows = screen.getAllByRole("radio");
    expect(windows.map((window) => window.textContent)).toEqual([
      es.meetingQuestion.window(2),
      es.meetingQuestion.window(5),
      es.meetingQuestion.window(10),
    ]);
    expect(windows[0]?.getAttribute("aria-checked")).toBe("true");
    await userEvent.type(
      screen.getByRole("textbox", { name: es.meetingQuestion.fieldLabel }),
      "¿Qué dijo Lucía?{Enter}",
    );
    expect(sent).toEqual([{ command: "meeting_ask", question: "¿Qué dijo Lucía?", minutes: 2 }]);
    expect(done()).toBe(1);
  });

  it("sends the chosen window and nothing while the question is empty", async () => {
    const { sent } = ask();
    const button = screen.getByRole("button", { name: es.meetingQuestion.send });
    expect((button as HTMLButtonElement).disabled).toBe(true);
    await userEvent.click(screen.getByRole("radio", { name: es.meetingQuestion.window(10) }));
    await userEvent.type(screen.getByRole("textbox"), "  ¿Y antes?  ");
    await userEvent.click(button);
    expect(sent).toEqual([{ command: "meeting_ask", question: "¿Y antes?", minutes: 10 }]);
  });

  it("says a question is too long, without its bytes, and does not send it", async () => {
    const { sent } = ask();
    await userEvent.type(screen.getByRole("textbox"), "ñ".repeat(21));
    expect(screen.getByRole("alert").textContent).toBe(es.meetingQuestion.tooLong);
    expect(
      (screen.getByRole("button", { name: es.meetingQuestion.send }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(sent).toEqual([]);
  });

  it("Escape closes the field without asking", async () => {
    const { sent, done } = ask();
    await userEvent.type(screen.getByRole("textbox"), "hola{Escape}");
    expect(done()).toBe(1);
    expect(sent).toEqual([]);
  });

  it("says beside the field whether «Oye Deimos» listens and why not", () => {
    ask({
      voiceStatus: { state: "not_listening", why: { cause: "macos_version", os_version: "15.4" } },
    });
    expect(
      screen.getByText(es.meetingQuestion.voice.off(es.meetingQuestion.voice.why.macosVersion)),
    ).toBeTruthy();
  });
});

describe("P9-12 review: what the panel says is true", () => {
  it("says a question that never reached Deimos was not asked, not that he did not answer", () => {
    const refused = card(
      question({ stage: "failed", sent: false, cause: { cause: "privacy_stopped" } }),
    );
    expect(refused.region.textContent).toContain(es.meetingQuestion.notSent);
    expect(refused.region.textContent).not.toContain(es.meetingQuestion.failed);
    refused.view.unmount();
    const unanswered = card(
      question({ stage: "failed", sent: true, cause: { cause: "turn_failed" } }),
    );
    expect(unanswered.region.textContent).toContain(es.meetingQuestion.failed);
  });

  it("shows the answer's web links as links that open through the panel", async () => {
    const opened: string[] = [];
    render(
      <QuestionCard
        openUrl={(url) => opened.push(url)}
        question={question({
          stage: "answered",
          answer:
            "Mira https://acme.example/acta. Y [el informe](https://acme.example/i), no [esto](javascript:alert(1)).",
        })}
        send={() => undefined}
      />,
    );
    const links = screen.getAllByRole("link");
    // A label never hides where it leads: its host goes beside it.
    expect(links.map((link) => link.textContent)).toEqual([
      "https://acme.example/acta",
      es.meetingQuestion.linkWithHost("el informe", "acme.example"),
    ]);
    await userEvent.click(links[0]!);
    await userEvent.click(
      screen.getByRole("link", {
        name: es.meetingQuestion.linkWithHost("el informe", "acme.example"),
      }),
    );
    expect(opened).toEqual(["https://acme.example/acta", "https://acme.example/i"]);
    expect(screen.getByText(/javascript:alert/)).toBeTruthy();
  });

  it("says whether «Oye Deimos» listens during the recording, and why not, without codes", () => {
    const voice = es.meetingQuestion.voice;
    expect(voiceStatusText(undefined)).toBeNull();
    expect(voiceStatusText({ state: "listening" })).toBe(voice.listening);
    expect(voiceStatusText({ state: "starting" })).toBeNull();
    expect(voiceStatusText({ state: "downloading", percent: 42 })).toBe(voice.downloading(42));
    const cases: [VoiceOffCause, string][] = [
      [{ cause: "privacy_stopped" }, voice.why.privacyStopped],
      [{ cause: "paused" }, voice.why.paused],
      [{ cause: "macos_version", os_version: "15.4" }, voice.why.macosVersion],
      [{ cause: "macos_version" }, voice.why.macosVersion],
      [{ cause: "locale_unsupported" }, voice.why.localeUnsupported],
      [{ cause: "unavailable", code: "otra" }, voice.why.unavailable],
      [{ cause: "failed", code: "assets_download_failed", detail: "sin red" }, voice.why.failed],
      [{ cause: "failed", code: "audio_format_unsupported" }, voice.why.failed],
      [{ cause: "failed", code: "analyzer_failed" }, voice.why.failed],
      [{ cause: "failed", code: "stream_not_found" }, voice.why.failed],
      [{ cause: "helper_disconnected" }, voice.why.helperDisconnected],
      [{ cause: "helper_without_phrase" }, voice.why.helperWithoutPhrase],
      [{ cause: "no_microphone" }, voice.why.noMicrophone],
    ];
    for (const [why, text] of cases) {
      const said = voiceStatusText({ state: "not_listening", why });
      expect(said).toBe(voice.off(text));
      expect(said).not.toMatch(/otra|sin red|stream_not_found|15\.4/);
    }
  });
});

describe("P9-12 review 2: a link says where it leads", () => {
  it("puts the host beside a Markdown label, also when the label is another address", () => {
    render(
      <QuestionCard
        question={question({
          stage: "answered",
          answer:
            "[Acta](https://docs.google.com/d/1) y [https://example.test](https://malo.example/x)",
        })}
        send={() => undefined}
      />,
    );
    expect(screen.getAllByRole("link").map((link) => link.textContent)).toEqual([
      es.meetingQuestion.linkWithHost("Acta", "docs.google.com"),
      es.meetingQuestion.linkWithHost("https://example.test", "malo.example"),
    ]);
  });
});
