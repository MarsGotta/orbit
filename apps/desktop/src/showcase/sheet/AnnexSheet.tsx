import { AssistantAvatar, type AssistantAvatarSize } from "../../design/AssistantAvatar";
import { Button } from "../../design/Button";
import { Icon, type IconName } from "../../design/Icon";
import { BeingRedone, ProgressCard } from "../../design/ProgressCard";
import { toggleTheme, useTheme } from "../../design/theme";
import { ViewNotice } from "../../design/ViewNotice";
import { EditorSection } from "./AnnexEditor";
import { AnswerActions, HeaderSection } from "./AnnexMenus";
import { MenusSection } from "./AnnexWord";
import sheet from "./DesignSheet.module.css";
import styles from "./AnnexSheet.module.css";

const ICONS: [IconName, string][] = [
  ["edit", "editar"],
  ["gear", "engranaje"],
  ["copy", "copiar"],
  ["listen", "escuchar"],
  ["stop", "parar"],
  ["download", "descargar"],
  ["expand", "ampliar"],
  ["conversation", "conversación"],
];
const SIZES: AssistantAvatarSize[] = [18, 20, 26, 28, 36, 40, 56];
const ASSISTANTS = [
  ["Deimos", 1],
  ["Phobos", 2],
  ["Ares", 3],
  ["Ío", 4],
] as const;

/**
 * The annex's sheet in the showcase: its two colours, its icons and its new
 * components, drawn by the components themselves, in the states the screens use.
 */
export function AnnexSheet() {
  const theme = useTheme();
  return (
    <main className={`${sheet.sheet} ${styles.annex}`}>
      <header className={sheet.header}>
        <div className={sheet.headerText}>
          <h1 className={sheet.title}>Componentes del anexo</h1>
          <p className={sheet.lead}>
            Lo nuevo para varios asistentes, el editor y la transcripción.
          </p>
        </div>
        <Button onClick={toggleTheme} size={32} variant="outline">
          <Icon name={theme === "dark" ? "themeLight" : "themeDark"} size={16} />
          {theme === "dark" ? "Ver en claro" : "Ver en oscuro"}
        </Button>
      </header>
      <div className={sheet.columns}>
        <ColorsAndIcons />
        <section className={sheet.section}>
          <h2 className={sheet.heading}>Botones de icono · con su tooltip</h2>
          <AnswerActions />
          <HeaderSection />
        </section>
      </div>
      <div className={sheet.columns}>
        <AvatarsSection />
        <NoticesSection />
      </div>
      <MenusSection />
      <EditorSection />
    </main>
  );
}

function ColorsAndIcons() {
  return (
    <section className={sheet.section}>
      <h2 className={sheet.heading}>Colores nuevos</h2>
      <p className={styles.sample}>
        Ya está en <span className={styles.wordHover}>Orvit</span>, y se ve{" "}
        <span className={styles.selected}>lo que has seleccionado</span>.
      </p>
      <p className={sheet.meta}>--o-wordHover · --o-sel</p>
      <h2 className={`${sheet.heading} ${styles.below}`}>Iconos nuevos</h2>
      <div className={styles.icons}>
        {ICONS.map(([name, label]) => (
          <div className={sheet.iconCell} key={name}>
            <Icon name={name} />
            <span className={sheet.meta}>{label}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

function AvatarsSection() {
  return (
    <section className={sheet.section}>
      <h2 className={sheet.heading}>Avatar de asistente</h2>
      {ASSISTANTS.map(([name, color]) => (
        <div className={`${sheet.row} ${styles.avatars}`} key={name}>
          {SIZES.map((size) => (
            <AssistantAvatar color={color} key={size} name={name} size={size} />
          ))}
          <span className={sheet.meta}>{name}</span>
        </div>
      ))}
    </section>
  );
}

function NoticesSection() {
  return (
    <section className={sheet.section}>
      <h2 className={sheet.heading}>Aviso dentro de una vista · tarjeta de progreso</h2>
      <div className={sheet.stack}>
        <ViewNotice
          actionLabel="Iniciar sesión en Claude"
          onAction={() => undefined}
          title="Inicia sesión otra vez en Claude"
        >
          Tus conversaciones con Ares siguen aquí; solo falta tu sesión para que pueda responder.
        </ViewNotice>
        <ViewNotice
          actionLabel="Iniciar sesión en Claude"
          onAction={() => undefined}
          title="Esperando a que inicies sesión en el navegador…"
          waiting
        >
          Se ha abierto tu navegador. Inicia sesión con tu cuenta de Claude y vuelve aquí.
        </ViewNotice>
        <ProgressCard percent={38} remaining="unos 3 min">
          Rehaciendo la transcripción…{" "}
          <span className={sheet.meta}>· puedes seguir con lo tuyo</span>
        </ProgressCard>
        <BeingRedone active>
          <p className={styles.sample}>
            <strong>Marta Ibáñez</strong> · Ya está en Orvit. Lo probamos ayer con tres
            recepcionistas.
          </p>
        </BeingRedone>
        <ProgressCard failed onClose={() => undefined}>
          No se pudo completar la nueva transcripción; se conserva la anterior.
        </ProgressCard>
      </div>
    </section>
  );
}
