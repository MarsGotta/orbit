import { useState } from "react";
import { Button } from "../../design/Button";
import { ConfirmCard } from "../../design/ConfirmCard";
import { EditMenu } from "../../design/EditMenu";
import { Field } from "../../design/Field";
import { Icon } from "../../design/Icon";
import { MultiSelect } from "../../design/MultiSelect";
import sheet from "./DesignSheet.module.css";
import styles from "./AnnexSheet.module.css";

const DICTIONARIES = [
  { id: "nubaria", name: "Nubaria", detail: "Revisión semanal y Comité · 24 palabras" },
  { id: "ribera", name: "Hoteles Ribera", detail: "Seguimiento del piloto · 11 palabras" },
  { id: "producto", name: "Producto", detail: "Ninguna serie · 8 palabras" },
];
const HERE = ["nubaria", "ribera"];

/** «Rehacer transcripción» with its confirmation, and a word low in a transcript, whose menu opens above. */
export function MenusSection() {
  const [redo, setRedo] = useState<HTMLButtonElement | null>(null);
  const [asking, setAsking] = useState(false);
  return (
    <section className={sheet.section}>
      <h2 className={sheet.heading}>
        Confirmación anclada · menú anclado de edición · selector de varios
      </h2>
      <div className={sheet.row}>
        <span className={styles.spacer} />
        <Button aria-expanded={asking} onClick={() => setAsking(true)} ref={setRedo} size={32}>
          <Icon name="refresh" size={15} />
          Rehacer transcripción
        </Button>
      </div>
      {asking && (
        <ConfirmCard
          anchor={redo}
          confirmLabel="Rehacer transcripción"
          note="Se mantienen los nombres que pusiste a cada persona. El resumen no cambia hasta que lo rehagas."
          onClose={() => setAsking(false)}
          onConfirm={() => setAsking(false)}
          title="¿Volver a transcribir la reunión?"
        >
          Orbit vuelve a escuchar la grabación (47 min) en español con Nubaria y Hoteles Ribera.
          Tarda unos 3 minutos y puedes seguir con lo tuyo.
        </ConfirmCard>
      )}
      <WordToCorrect />
    </section>
  );
}

function WordToCorrect() {
  const [word, setWord] = useState<HTMLButtonElement | null>(null);
  const [open, setOpen] = useState(false);
  const [fix, setFix] = useState("Orbit");
  const [chosen, setChosen] = useState(HERE);
  const options = DICTIONARIES.map((option) =>
    HERE.includes(option.id) ? { ...option, tag: "esta reunión" } : option,
  );
  return (
    <p className={`${styles.sample} ${styles.transcript}`}>
      <strong>Marta Ibáñez</strong> · Ya está en{" "}
      <button
        aria-expanded={open}
        className={styles.word}
        onClick={() => setOpen(true)}
        ref={setWord}
        tabIndex={-1}
        type="button"
      >
        Orvit
      </button>
      . Lo probamos ayer con tres recepcionistas y lo terminaron todas sin ayuda.
      {open && (
        <EditMenu
          anchor={word}
          canSave={fix.trim() !== "" && fix.trim() !== "Orvit" && chosen.length > 0}
          onClose={() => setOpen(false)}
          onSave={() => setOpen(false)}
          saveLabel="Corregir y añadir"
          subtitle="Sale 3 veces en esta reunión."
          title="Corregir «Orvit»"
          width={420}
        >
          <Field label="Debe decir" onChange={(event) => setFix(event.target.value)} value={fix} />
          <MultiSelect
            createLabel="+ Crear un diccionario nuevo"
            label="Se añade a · puedes elegir varios"
            onChange={setChosen}
            onCreate={() => undefined}
            options={options}
            placeholder="Elige uno o varios"
            selected={chosen}
          />
        </EditMenu>
      )}
    </p>
  );
}
