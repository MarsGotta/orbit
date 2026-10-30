import type { Scenario, Step } from "../scenario";
import { distillationPort } from "../data/distillation";
const note: Step = { click: 'role=button[name="Cedar Pilot · 2 cambios"]' };
const decide: Step = { click: 'role=radio[name="Usar 2026-11-02"]' };
const base = (id: string, state: string, steps: Step[] = [], applied = false): Scenario => ({
  id,
  state,
  steps,
  view: "Notas",
  description: "Propuesta inventada de Cedar: revisión, ediciones y decisiones antes de escribir.",
  route: "#/vault/distill/00000000-0000-4000-8000-000000000312",
  ready: 'h1:has-text("Cedar review")',
  vault: distillationPort(applied ? "applied" : "proposed", id === "distill-applying"),
});
export const distillationScenarios: Scenario[] = [
  base("distill-conflict", "Hay que decidir"),
  base("distill-change", "Un cambio", [note]),
  base("distill-edit", "Editar antes de aplicar", [
    note,
    { click: 'role=button[name="Editar antes de aplicar"]' },
  ]),
  base("distill-decided", "Fecha decidida", [decide]),
  base("distill-excluded", "Sin un cambio", [
    decide,
    { click: 'role=checkbox[name="Incluir Eli Brook"]' },
  ]),
  base("distill-applying", "Aplicando", [
    decide,
    { click: 'role=button[name="Aplicar 3 cambios"]' },
  ]),
  base("distill-applied", "Aplicado", [], true),
  {
    id: "vault-pending",
    state: "Para revisar",
    view: "Notas",
    description: "Propuesta pendiente en Inicio y en la columna.",
    route: "#/vault",
    ready: 'role=link[name="Revisar los cambios"]',
    vault: distillationPort(),
  },
];
