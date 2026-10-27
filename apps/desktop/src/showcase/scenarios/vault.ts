import type { Scenario } from "../scenario";

const reading = (id: string, state: string, route: string, ready: string): Scenario => ({
  id,
  state,
  route,
  ready,
  view: "Notas",
  description: "Vault de ejemplo inventado: lectura sin cuenta, Mac ni servidor de sincronización.",
});
export const vaultScenarios: Scenario[] = [
  reading("vault-home", "Inicio", "#/vault", "h2:has-text('Proyectos')"),
  reading("vault-connect", "Conectar tu vault", "#/vault", "text=Tus notas, en Markdown"),
  reading(
    "vault-person",
    "Ficha de persona",
    "#/vault/n/personas%2Fjavier-ortega.md",
    "h1:has-text('Javier Ortega')",
  ),
  reading(
    "vault-markdown",
    "Markdown",
    "#/vault/n/personas%2Fjavier-ortega.md?md=1",
    "text=Propiedades (YAML) · OKF 0.2 · Solo lectura",
  ),
  reading(
    "vault-project",
    "Proyecto",
    "#/vault/n/proyectos%2Fpiloto-hoteles-ribera.md",
    "svg[aria-label='Diagrama de dependencias']",
  ),
  reading("vault-tasks", "Tareas", "#/vault/tasks", "h1:has-text('Tareas')"),
  reading("vault-map", "Mapa", "#/vault/map", "svg[aria-label='Mapa de enlaces entre notas']"),
  reading("vault-settings", "Ajustes · Notas", "#/settings/notas", "text=Modelo de mantenimiento"),
];
