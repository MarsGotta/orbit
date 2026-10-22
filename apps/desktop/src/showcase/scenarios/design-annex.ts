import type { Scenario } from "../scenario";

const SHEET = {
  view: "Sistema de diseño",
  sheet: "annex",
  size: { width: 1440, height: 1500 },
  route: "#/",
  ready: "text=Componentes del anexo",
} as const;

const PENCIL = 'role=button[name="Editar nombre y descripción de Phobos"]';

/** The annex's components: at rest, echoing, and each anchored window open. */
export const designAnnexScenarios: Scenario[] = [
  {
    ...SHEET,
    id: "anexo-componentes",
    state: "Componentes del anexo",
    description:
      "Los dos colores nuevos, los iconos nuevos, los botones de icono, el avatar de asistente en sus siete medidas, el aviso dentro de una vista y la tarjeta de progreso.",
  },
  {
    ...SHEET,
    id: "anexo-eco-y-tooltip",
    state: "Copiado, sonando y un tooltip",
    description:
      "Copiar pasa a ✓ «Copiado», Escuchar a «Parar» y el tooltip de «Volver a preguntar» sale encima al pasar el ratón.",
    steps: [
      { click: 'role=button[name="Escuchar"]' },
      { click: 'role=button[name="Copiar"]' },
      { hover: 'role=button[name="Volver a preguntar"]' },
    ],
  },
  {
    ...SHEET,
    id: "anexo-lapiz",
    state: "Menú del lápiz",
    description:
      "El menú anclado de edición, de 400, debajo del lápiz; «Guardar» apagado hasta que hay un cambio válido.",
    steps: [{ click: PENCIL }],
  },
  {
    ...SHEET,
    id: "anexo-lapiz-nombre-repetido",
    state: "Menú del lápiz · nombre repetido",
    description: "Un nombre que ya tiene otro asistente no se guarda y dice por qué.",
    steps: [{ click: PENCIL }, { fill: 'role=textbox[name="Nombre"]', text: "Deimos" }],
  },
  {
    ...SHEET,
    id: "anexo-confirmacion",
    state: "Confirmación anclada",
    description:
      "«Rehacer transcripción» pregunta junto al botón; el principal repite el mismo verbo.",
    steps: [{ click: 'role=button[name="Rehacer transcripción"]' }],
  },
  {
    ...SHEET,
    id: "anexo-corregir-palabra",
    state: "Corregir una palabra · selector de varios",
    description:
      "El menú de 420 de una palabra abajo en la transcripción se abre encima, con los diccionarios elegidos y la lista abierta.",
    steps: [{ click: 'role=button[name="Orvit"]' }, { click: "role=button[name=/Se añade a/]" }],
  },
];
