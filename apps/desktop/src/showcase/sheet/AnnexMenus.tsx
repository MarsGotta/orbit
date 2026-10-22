import { useState } from "react";
import { AssistantAvatar } from "../../design/AssistantAvatar";
import { Button } from "../../design/Button";
import { EditMenu } from "../../design/EditMenu";
import { useFeedback } from "../../design/feedback";
import { Field } from "../../design/Field";
import { Icon } from "../../design/Icon";
import { IconButton } from "../../design/IconButton";
import sheet from "./DesignSheet.module.css";
import styles from "./AnnexSheet.module.css";

const TAKEN = ["deimos", "ares"];

/** Under an answer: Copiar and Escuchar with their echo, at rest and echoing. */
export function AnswerActions() {
  const copy = useFeedback();
  const [playing, setPlaying] = useState(false);
  return (
    <div className={sheet.stack}>
      <p className={`${styles.sample} ${styles.answer}`}>
        La media de la semana es de 112 € por noche; Casa Ribera es la más cara.
      </p>
      <div className={sheet.row}>
        <IconButton
          doneLabel="Copiado"
          feedback={copy.feedback}
          icon="copy"
          label="Copiar"
          onClick={() => copy.track(Promise.resolve())}
          size={28}
        />
        <IconButton
          active={playing ? { icon: "stop", label: "Parar" } : undefined}
          icon="listen"
          label="Escuchar"
          onClick={() => setPlaying(!playing)}
          size={28}
        />
        <IconButton icon="refresh" label="Volver a preguntar" size={28} />
        <IconButton icon="edit" label="Editar y copiar" size={28} />
        <span className={sheet.meta}>· mientras copia y mientras suena:</span>
        <IconButton doneLabel="Copiado" feedback="done" icon="copy" label="Copiar" size={28} />
        <IconButton
          active={{ icon: "stop", label: "Parar" }}
          icon="listen"
          label="Escuchar"
          size={28}
        />
      </div>
    </div>
  );
}

/** A conversation's header: the pencil opens its menu below; the tooltips go below too. */
export function HeaderSection() {
  const [anchor, setAnchor] = useState<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);
  return (
    <div className={styles.header}>
      <AssistantAvatar color={2} name="Phobos" size={26} />
      <span className={styles.headerName}>Phobos</span>
      <IconButton
        aria-expanded={open}
        aria-haspopup="dialog"
        aria-label="Editar nombre y descripción de Phobos"
        icon="edit"
        label="Editar nombre y descripción"
        onClick={() => setOpen(!open)}
        ref={setAnchor}
        size={30}
        tip="below"
      />
      <IconButton icon="gear" label="Ajustes de Phobos" size={30} tip="below" />
      <span className={styles.spacer} />
      <IconButton icon="phone" label="Llamar a Phobos" size={34} tip="below" />
      <IconButton icon="themeLight" label="Tema claro" size={34} tip="below" />
      {open && <PencilMenu anchor={anchor} onClose={() => setOpen(false)} />}
    </div>
  );
}

/** Phobos's pencil: a name that is empty or taken is not saved. */
function PencilMenu({ anchor, onClose }: { anchor: HTMLElement | null; onClose: () => void }) {
  const [name, setName] = useState("Phobos");
  const [about, setAbout] = useState("Código, scripts y datos");
  const clean = name.trim();
  const taken = TAKEN.includes(clean.toLocaleLowerCase("es"));
  const changed = clean !== "Phobos" || about !== "Código, scripts y datos";
  const hint =
    clean === "" ? "Ponle un nombre: es como le llamas." : `Le llamas con «Oye ${clean}».`;
  return (
    <EditMenu
      anchor={anchor}
      canSave={clean !== "" && !taken && changed}
      error={taken ? "Ya tienes otro asistente que se llama así." : undefined}
      extra={
        <Button size={32} variant="ghost">
          <Icon name="gear" size={15} />
          Más ajustes
        </Button>
      }
      leading={<AssistantAvatar color={2} name={clean || "Phobos"} size={40} />}
      onClose={onClose}
      onSave={onClose}
      subtitle="Funciona con Codex, con tu suscripción de ChatGPT"
      title="Editar a Phobos"
      width={400}
    >
      <Field
        help={hint}
        label="Nombre"
        maxLength={24}
        onChange={(e) => setName(e.target.value)}
        value={name}
      />
      <label className={sheet.meta}>
        Descripción
        <textarea
          className={styles.textarea}
          maxLength={90}
          onChange={(event) => setAbout(event.target.value)}
          value={about}
        />
      </label>
    </EditMenu>
  );
}
