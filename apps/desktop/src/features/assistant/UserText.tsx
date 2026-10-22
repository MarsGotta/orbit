import { es } from "../../i18n/es";
import { CopyButton } from "./CopyButton";
import styles from "./MessageActions.module.css";

/** A message of the user, as they wrote it, with «Copiar». */
export function UserText({ text }: { text: string }) {
  return (
    <>
      <p>{text}</p>
      <div className={styles.actions} data-align="end">
        <CopyButton label={es.chatActions.copyMessage} text={() => text} />
      </div>
    </>
  );
}
