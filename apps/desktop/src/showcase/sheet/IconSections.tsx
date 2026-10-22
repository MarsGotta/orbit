import { Icon, iconNames } from "../../design/Icon";
import { Skeleton } from "../../design/Skeleton";
import { Loading, Spinner } from "../../design/Spinner";
import styles from "./DesignSheet.module.css";

export function IconsSection() {
  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Iconos · trazo fino, sin relleno</h2>
      <div className={styles.icons}>
        {iconNames.map((name) => (
          <div className={styles.iconCell} key={name}>
            <Icon name={name} />
            <span className={styles.meta}>{name}</span>
          </div>
        ))}
      </div>
      <p className={styles.caption}>
        20 en la barra lateral, 16 en botones, 14 en metadatos. Siempre en gris; violeta solo en lo
        seleccionado.
      </p>
    </section>
  );
}

const MOVEMENTS = [
  ["Late", "Solo el punto de «Grabando»: así se sabe, sin mirar, que algo escucha."],
  ["Gira", "Algo en curso que terminará solo: transcribir, guardar, emparejar."],
  ["Se rellena", "Contenido que está llegando del hub."],
  ["Aparece", "Ventanas, menús y avisos entran con un fundido corto, sin rebotes."],
] as const;

export function MotionSection() {
  const samples = [
    <span className={styles.pulse} key="late" />,
    <Spinner key="gira" />,
    <div key="rellena" style={{ width: 36 }}>
      <Skeleton height={10} widths={["100%"]} />
    </div>,
    <span className={styles.meta} key="aparece">
      180 ms
    </span>,
  ];
  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Movimiento · casi nada se mueve</h2>
      {MOVEMENTS.map(([name, use], index) => (
        <div className={styles.motion} key={name}>
          <span aria-hidden="true" className={styles.motionSample}>
            {samples[index]}
          </span>
          <div>
            <div className={styles.name} style={{ marginTop: 0 }}>
              {name}
            </div>
            <div className={styles.meta}>{use}</div>
          </div>
        </div>
      ))}
      <p className={styles.caption}>Cargando · siempre con forma, nunca en blanco</p>
      <div aria-busy="true" className={styles.stack} style={{ maxWidth: 460 }}>
        <div style={{ width: "100%" }}>
          <Skeleton widths={["60%", "92%", "78%"]} />
        </div>
        <Loading>Leyendo tus notas del hub…</Loading>
      </div>
      <p className={styles.caption}>
        Con «Reducir movimiento» activado en macOS, todo queda quieto.
      </p>
    </section>
  );
}
