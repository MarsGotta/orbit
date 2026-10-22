import { useState } from "react";
import { Button, RecordButton } from "../../design/Button";
import { Checkbox } from "../../design/Checkbox";
import { useFeedback } from "../../design/feedback";
import { Field } from "../../design/Field";
import { Icon } from "../../design/Icon";
import { RadioList } from "../../design/RadioList";
import { Segmented } from "../../design/Segmented";
import { StatusPill } from "../../design/StatusPill";
import { Switch } from "../../design/Switch";
import { Tabs } from "../../design/Tabs";
import styles from "./DesignSheet.module.css";

// A press in the sheet that the hub never answers: the control keeps waiting.
const never = () => new Promise<void>(() => undefined);
const later = () => new Promise<void>((done) => window.setTimeout(done, 900));

export function ControlsSection() {
  const save = useFeedback();
  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Componentes</h2>
      <p className={styles.caption}>Botones · una sola principal por pantalla</p>
      <div className={styles.row}>
        <Button size={36} variant="accent">
          Principal
        </Button>
        <Button size={36}>Secundario</Button>
        <Button size={36} variant="ghost">
          Discreto
        </Button>
        <RecordButton shortcut="⌃⌥M">Grabar reunión</RecordButton>
        <Button size={36} variant="dangerSoft">
          Parar todo
        </Button>
        <Button size={36} variant="danger">
          Empezar a grabar
        </Button>
        <Button aria-label="Cerrar" size={36} variant="icon">
          <Icon name="close" size={16} />
        </Button>
      </div>
      <p className={styles.caption}>Alturas · 28 en barras y avisos, 32 en tarjetas, 36, 40</p>
      <div className={styles.row}>
        {([28, 32, 36, 40] as const).map((size) => (
          <Button key={size} size={size}>
            {size}
          </Button>
        ))}
        <Button
          disabledReason="Deimos está respondiendo · tu texto se queda"
          size={36}
          variant="accent"
        >
          Enviar
        </Button>
      </div>
      <p className={styles.caption}>Todo responde · pulsa «Guardar»</p>
      <div className={styles.row}>
        <Button feedback={save.feedback} onClick={() => save.track(later())} size={36}>
          Guardar
        </Button>
        <Button feedback="pending" size={36}>
          Guardar
        </Button>
        <Button feedback="done" size={36}>
          Guardar
        </Button>
      </div>
      <SheetPills />
    </section>
  );
}

function SheetPills() {
  return (
    <>
      <p className={styles.caption}>Estados · el color solo aparece cuando pasa algo</p>
      <div className={styles.row}>
        <StatusPill kind="recording">Grabando · 12:34</StatusPill>
        <StatusPill kind="working">Transcribiendo…</StatusPill>
        <StatusPill kind="done">✓ Nota lista</StatusPill>
        <StatusPill kind="warning">Falta un permiso</StatusPill>
      </div>
    </>
  );
}

export function ChoicesSection() {
  const [name, setName] = useState("Lucía");
  const [login, setLogin] = useState(true);
  const [view, setView] = useState("week");
  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Campos y opciones</h2>
      <div className={styles.stack}>
        <Field
          help="Así te saluda Orbit cada mañana."
          label="Tu nombre"
          onChange={(event) => setName(event.target.value)}
          saved
          style={{ width: 280 }}
          value={name}
        />
        <Field label="Buscar" placeholder="Notas, personas o una orden…" style={{ width: 280 }} />
        <Switch checked={login} label="Abrir Orbit al encender el Mac" onCheckedChange={setLogin}>
          Abrir Orbit al encender el Mac
        </Switch>
        <Switch
          checked={false}
          label="Micrófono siempre listo"
          onCheckedChange={never}
          waitsForSystem
        >
          Micrófono siempre listo
        </Switch>
        <Checkbox checked={false} onCheckedChange={() => undefined}>
          Revisar el contrato con Nubaria
        </Checkbox>
        <Checkbox checked={false} onCheckedChange={never}>
          Enviar el resumen a Sara
        </Checkbox>
        <Segmented
          label="Vista"
          onChange={setView}
          options={[
            { value: "day", label: "Día" },
            { value: "week", label: "Semana" },
            { value: "month", label: "Mes" },
          ]}
          value={view}
        />
      </div>
      <SheetRadios />
      <SheetTabs />
    </section>
  );
}

function SheetRadios() {
  const [audio, setAudio] = useState("system");
  return (
    <>
      <p className={styles.caption}>Lista de opciones</p>
      <RadioList
        label="Qué audio grabar"
        name="sheet-audio"
        onChange={setAudio}
        options={[
          {
            value: "system",
            label: "Audio del sistema",
            help: "Lo que suena en el Mac y tu micrófono. Para videollamadas",
          },
          { value: "mic", label: "Solo mi micrófono", help: "Para una reunión en persona" },
        ]}
        value={audio}
      />
    </>
  );
}

function SheetTabs() {
  const [tab, setTab] = useState("summary");
  return (
    <>
      <p className={styles.caption}>Pestañas</p>
      <Tabs
        label="Nota"
        onSelect={setTab}
        selected={tab}
        tabs={[
          { id: "summary", label: "Resumen" },
          { id: "actions", label: "Acciones · 3" },
          { id: "transcript", label: "Transcripción" },
        ]}
      >
        <p className={styles.meta}>
          {tab === "summary" ? "El resumen de la reunión." : "Lo que hay en esta pestaña."}
        </p>
      </Tabs>
    </>
  );
}
