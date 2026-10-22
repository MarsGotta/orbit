import { Button } from "../../design/Button";
import { Icon } from "../../design/Icon";
import { toggleTheme, useTheme } from "../../design/theme";
import { ColorSection, TypeSection } from "./BaseSections";
import { ChoicesSection, ControlsSection } from "./ControlSections";
import { IconsSection, MotionSection } from "./IconSections";
import { FloatingSection, ScreenPartsSection } from "./PartSections";
import styles from "./DesignSheet.module.css";

/**
 * The design system's sheet in the showcase: the tokens and every component
 * in the states the screens use, drawn by the components themselves.
 */
export function DesignSheet() {
  const theme = useTheme();
  return (
    <main className={styles.sheet}>
      <header className={styles.header}>
        <div className={styles.headerText}>
          <div className={styles.brand}>
            <Icon name="logo" size={18} />
            <span>Orbit</span>
          </div>
          <h1 className={styles.title}>Sistema de diseño</h1>
          <p className={styles.lead}>
            Menos color y menos adornos: en cada pantalla hay un solo sitio donde mirar.
          </p>
        </div>
        <Button onClick={toggleTheme} size={32} variant="outline">
          <Icon name={theme === "dark" ? "themeLight" : "themeDark"} size={16} />
          {theme === "dark" ? "Ver en claro" : "Ver en oscuro"}
        </Button>
      </header>
      <ColorSection theme={theme} />
      <div className={styles.columns}>
        <TypeSection />
        <ControlsSection />
      </div>
      <div className={styles.columns}>
        <ChoicesSection />
        <ScreenPartsSection />
      </div>
      <FloatingSection />
      <div className={styles.columns}>
        <IconsSection />
        <MotionSection />
      </div>
    </main>
  );
}
