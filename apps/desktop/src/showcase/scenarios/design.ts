import type { Scenario } from "../scenario";

export const designScenarios: Scenario[] = [
  {
    id: "sistema-de-diseno",
    view: "Sistema de diseño",
    state: "Componentes en sus estados",
    description:
      "Los tokens de los dos temas y cada componente de la app en los estados que usan las pantallas: esperando a macOS y guardando tras pulsar.",
    sheet: true,
    size: { width: 1440, height: 2920 },
    route: "#/",
    ready: "text=Sistema de diseño",
    steps: [
      { click: 'role=switch[name="Micrófono siempre listo"]' },
      { click: 'role=checkbox[name="Enviar el resumen a Sara"]' },
    ],
  },
];
