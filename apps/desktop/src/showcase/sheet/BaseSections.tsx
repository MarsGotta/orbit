import type { Theme } from "../../design/theme";
import styles from "./DesignSheet.module.css";

type Swatch = [name: string, token: string, dark: string, light: string, use: string];

const NEUTRALS: Swatch[] = [
  ["Fondo", "bg", "#131316", "#F7F7F8", "detrás de todo"],
  ["Lateral", "side", "#161619", "#F1F1F3", "barra lateral"],
  ["Tarjeta", "surface", "#1B1B1F", "#FFFFFF", "tarjetas y paneles"],
  ["Elevada", "raised", "#232328", "#F4F4F5", "seleccionado"],
  ["Línea", "line2", "#34343B", "#D4D4D8", "bordes y separadores"],
  ["Texto", "ink", "#F4F4F5", "#18181B", "texto principal"],
  ["Texto 2", "ink2", "#A1A1AA", "#52525B", "secundario e iconos"],
  ["Texto 3", "ink3", "#8E8E97", "#6B6B74", "ayudas y horas"],
];

const SIGNALS: Swatch[] = [
  ["Violeta · acción", "acc", "#6B5BE0", "#5B4BDB", "botón principal"],
  ["Violeta · texto", "accText", "#A99EFA", "#5B4BDB", "enlaces, seleccionado"],
  ["Rojo", "rec", "#D13D44", "#C93A40", "grabando o error"],
  ["Ámbar", "warn", "#F5B544", "#9A5B00", "falta algo"],
  ["Verde", "ok", "#4CC9A0", "#0F7A5A", "hecho"],
  ["Azul", "info", "#6CB4FF", "#1F6FB8", "aviso neutro"],
];

const CALENDARS: Swatch[] = [
  ["Trabajo", "c1", "#9D90F5", "#6B5BE0", ""],
  ["Clientes", "c2", "#6CB4FF", "#2B7CD0", ""],
  ["Equipo", "c3", "#4CC9A0", "#13936B", ""],
  ["Personal", "c4", "#E8B04A", "#B07A12", ""],
];

const hex = (swatch: Swatch, theme: Theme) => (theme === "dark" ? swatch[2] : swatch[3]);

export function ColorSection({ theme }: { theme: Theme }) {
  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Color · {theme === "dark" ? "tema oscuro" : "tema claro"}</h2>
      <div className={styles.columns}>
        <div style={{ flex: "none", width: 680 }}>
          <p className={styles.caption}>Neutros · casi todo es esto</p>
          <div className={styles.grid}>
            {NEUTRALS.map((swatch) => (
              <div key={swatch[1]}>
                <div className={styles.swatch} style={{ background: `var(--o-${swatch[1]})` }} />
                <div className={styles.name}>{swatch[0]}</div>
                <div className={styles.meta}>
                  {hex(swatch, theme)} · {swatch[4]}
                </div>
              </div>
            ))}
          </div>
        </div>
        <Signals
          swatches={SIGNALS}
          theme={theme}
          title="Acción y estado · solo cuando hace falta"
        />
        <Signals swatches={CALENDARS} theme={theme} title="Calendarios · puntos pequeños" dots />
      </div>
    </section>
  );
}

function Signals(props: { swatches: Swatch[]; theme: Theme; title: string; dots?: boolean }) {
  return (
    <div style={{ minWidth: 0, flex: 1 }}>
      <p className={styles.caption}>{props.title}</p>
      {props.swatches.map((swatch) => (
        <div className={styles.signal} key={swatch[1]}>
          <span
            aria-hidden="true"
            className={props.dots ? styles.calDot : styles.chip}
            style={{ background: `var(--o-${swatch[1]})` }}
          />
          <div>
            <div className={styles.name} style={{ marginTop: 0 }}>
              {swatch[0]}
            </div>
            <div className={styles.meta}>
              {hex(swatch, props.theme)}
              {swatch[4] && ` · ${swatch[4]}`}
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

const TYPE: [sample: string, spec: string, font: string][] = [
  ["Buenos días, Lucía", "28 · seminegrita · título de página", "var(--o-text-title)"],
  ["Revisión del diseño de reservas", "20 · seminegrita · título de una nota", "var(--o-text-h2)"],
  ["Agenda de hoy", "15 · seminegrita · título de tarjeta", "var(--o-text-h3)"],
  ["Se elige el pago en una sola pantalla.", "14 · normal · texto", "var(--o-text-body)"],
  ["Unirse y grabar", "13 · media · botones y listas", "var(--o-text-control)"],
  ["11:00 – 11:45 · 4 personas", "12 · normal · ayudas y horas", "var(--o-text-meta)"],
  ["09:30", "11 · media · horas del calendario; nada más pequeño", "var(--o-text-tiny)"],
];

export function TypeSection() {
  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Tipografía · solo Inter</h2>
      {TYPE.map(([sample, spec, font]) => (
        <div className={styles.typeRow} key={spec}>
          <div style={{ font, fontVariantNumeric: "tabular-nums" }}>{sample}</div>
          <div className={styles.meta}>{spec}</div>
        </div>
      ))}
      <div className={styles.typeRow}>
        <div className={styles.mono}>resumen-semanal</div>
        <div className={styles.meta}>JetBrains Mono 13 · solo código y nombres de habilidades</div>
      </div>
    </section>
  );
}
