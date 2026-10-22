import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, expect, it } from "vitest";
import { Field } from "./Field";
import { RadioList } from "./RadioList";
import { Segmented } from "./Segmented";
import { Tabs } from "./Tabs";

afterEach(cleanup);

const views = [
  { value: "day", label: "Día" },
  { value: "week", label: "Semana" },
  { value: "month", label: "Mes" },
];

function View() {
  const [value, setValue] = useState("week");
  return <Segmented label="Vista" onChange={setValue} options={views} value={value} />;
}

it("the segmented control is a radio group the arrows move through", () => {
  render(<View />);
  expect(screen.getByRole("radiogroup", { name: "Vista" })).toBeTruthy();
  const week = screen.getByRole("radio", { name: "Semana" });
  expect(week.getAttribute("aria-checked")).toBe("true");
  expect(week.tabIndex).toBe(0);
  expect(screen.getByRole("radio", { name: "Día" }).tabIndex).toBe(-1);
  fireEvent.keyDown(week, { key: "ArrowRight" });
  const month = screen.getByRole("radio", { name: "Mes" });
  expect(month.getAttribute("aria-checked")).toBe("true");
  expect(document.activeElement).toBe(month);
  fireEvent.keyDown(month, { key: "ArrowRight" });
  expect(screen.getByRole("radio", { name: "Día" }).getAttribute("aria-checked")).toBe("true");
  fireEvent.click(screen.getByRole("radio", { name: "Semana" }));
  expect(screen.getByRole("radio", { name: "Semana" }).getAttribute("aria-checked")).toBe("true");
});

function Sections() {
  const [tab, setTab] = useState("summary");
  return (
    <Tabs
      label="Nota"
      onSelect={setTab}
      selected={tab}
      tabs={[
        { id: "summary", label: "Resumen" },
        { id: "actions", label: "Acciones · 3" },
      ]}
    >
      {tab === "summary" ? "El resumen" : "Las acciones"}
    </Tabs>
  );
}

it("tabs mark the chosen one, move with the arrows and label their panel", () => {
  render(<Sections />);
  const summary = screen.getByRole("tab", { name: "Resumen" });
  expect(summary.getAttribute("aria-selected")).toBe("true");
  expect(screen.getByRole("tabpanel", { name: "Resumen" }).textContent).toBe("El resumen");
  fireEvent.keyDown(summary, { key: "ArrowRight" });
  const actions = screen.getByRole("tab", { name: "Acciones · 3" });
  expect(actions.getAttribute("aria-selected")).toBe("true");
  expect(document.activeElement).toBe(actions);
  expect(screen.getByRole("tabpanel", { name: "Acciones · 3" }).textContent).toBe("Las acciones");
});

it("the radio list is real radios, each with its help", () => {
  function Audio() {
    const [value, setValue] = useState("system");
    return (
      <RadioList
        label="Qué audio grabar"
        name="audio"
        onChange={setValue}
        options={[
          {
            value: "system",
            label: "Audio del sistema",
            help: "Lo que suena en el Mac y tu micrófono",
          },
          { value: "mic", label: "Solo mi micrófono", help: "Para una reunión en persona" },
        ]}
        value={value}
      />
    );
  }
  render(<Audio />);
  expect(screen.getByRole("radiogroup", { name: "Qué audio grabar" })).toBeTruthy();
  const mic = screen.getByRole("radio", { name: /Solo mi micrófono/ }) as HTMLInputElement;
  fireEvent.click(mic);
  expect(mic.checked).toBe(true);
});

it("a field is labelled above, helped below, and says «✓ Guardado» once saved", () => {
  render(<Field help="Así te saluda Orbit cada mañana." label="Tu nombre" saved />);
  const input = screen.getByLabelText("Tu nombre");
  expect(input.getAttribute("aria-describedby")).toBeTruthy();
  const help = document.getElementById(input.getAttribute("aria-describedby") ?? "");
  expect(help?.textContent).toBe("Así te saluda Orbit cada mañana.");
  expect(screen.getByText("✓ Guardado")).toBeTruthy();
});
