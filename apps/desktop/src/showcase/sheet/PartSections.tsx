import { AttentionRow } from "../../design/AttentionRow";
import { Button } from "../../design/Button";
import { CalendarEvent, type CalendarEventProps } from "../../design/CalendarEvent";
import { Card } from "../../design/Card";
import { Dialog } from "../../design/Dialog";
import { EmptyState } from "../../design/EmptyState";
import { Field } from "../../design/Field";
import { Popover } from "../../design/Popover";
import { SidebarItem, type SidebarItemProps } from "../../design/SidebarItem";
import { Toast } from "../../design/Toast";
import styles from "./DesignSheet.module.css";

const close = () => undefined;

const SECTIONS: SidebarItemProps[] = [
  { active: true, href: "#hoy", icon: "briefing", label: "Hoy" },
  { active: false, href: "#calendario", icon: "calendar", label: "Calendario" },
  { active: false, href: "#reuniones", icon: "meetings", label: "Reuniones", meta: "1 en curso" },
  {
    active: false,
    alert: { tone: "rec", text: "Te necesita" },
    href: "#asistente",
    icon: "assistant",
    label: "Asistente",
  },
  {
    active: false,
    alert: { tone: "warn", text: "Falta 1" },
    href: "#ajustes",
    icon: "settings",
    label: "Ajustes",
  },
];

// 48 px an hour: half an hour is 24 px.
const EVENTS: CalendarEventProps[] = [
  { color: 1, height: 24, time: "09:00 – 09:30", title: "Revisión semanal del equipo" },
  { color: 2, height: 48, time: "10:00 – 11:00", title: "Hoteles Ribera · propuesta" },
  { color: 3, height: 48, selected: true, time: "11:30 – 12:30", title: "Diseño de reservas" },
  {
    color: 4,
    height: 64,
    response: { kind: "unanswered", label: "Sin responder" },
    time: "13:00 – 14:00",
    title: "Comida con Nubaria",
  },
  {
    color: 2,
    height: 64,
    response: { kind: "tentative", label: "Quizá" },
    time: "16:00 – 17:00",
    title: "Taller de pagos",
  },
  { color: 1, height: 48, past: true, time: "08:00 – 09:00", title: "Correo y agenda" },
];

export function ScreenPartsSection() {
  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Piezas de las pantallas</h2>
      <div className={styles.columns} style={{ gap: 24 }}>
        <nav aria-label="Secciones" className={styles.sidebar}>
          {SECTIONS.map((item) => (
            <SidebarItem key={item.href} {...item} />
          ))}
        </nav>
        <div style={{ minWidth: 0, flex: 1 }}>
          <Card>
            <EmptyState
              action={
                <Button size={36} variant="accent">
                  Grabar reunión
                </Button>
              }
              icon="notes"
              title="Aún no hay notas"
            >
              Graba tu primera reunión y Orbit escribirá la nota por ti.
            </EmptyState>
          </Card>
        </div>
      </div>
      <p className={styles.caption}>Fila de atención</p>
      <AttentionRow
        action={<Button size={32}>Abrir Ajustes del Sistema</Button>}
        help="Sin él, Orbit no puede grabar tu voz."
        title="Falta el permiso de micrófono"
      />
      <p className={styles.caption}>
        Eventos del calendario · el tinte de su calendario y un punto
      </p>
      <div className={styles.events}>
        {EVENTS.map((event) => (
          <CalendarEvent key={event.title} {...event} />
        ))}
      </div>
    </section>
  );
}

export function FloatingSection() {
  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Lo que flota · avisos, ventanas y menús, con sombra</h2>
      <div className={styles.columns} style={{ gap: 24 }}>
        <SheetToasts />
        <SheetDialog />
        <SheetPopover />
      </div>
    </section>
  );
}

function SheetToasts() {
  return (
    <div className={styles.stack} style={{ width: 360, flex: "none", gap: 8 }}>
      <Toast
        action={<button type="button">Abrir nota</button>}
        embedded
        message="Reunión guardada"
        onClose={close}
        persistent
      />
      <Toast embedded kind="working" message="Transcribiendo tu nota de voz…" onClose={close} />
      <Toast
        detail="Orbit no puede grabar tu voz sin él."
        embedded
        kind="fixable"
        message="Falta el permiso de micrófono"
        onClose={close}
      />
      <Toast
        action={
          <>
            <Button size={28} variant="accent">
              Reintentar
            </Button>
            <Button size={28}>Copiar el código</Button>
          </>
        }
        detail="Vuelve a intentarlo; si se repite, comparte el código."
        embedded
        kind="failed"
        message="No se pudo guardar la nota"
        onClose={close}
      />
    </div>
  );
}

function SheetDialog() {
  return (
    <div className={styles.frame} style={{ height: 380, minWidth: 0, flex: 1 }}>
      <Dialog
        footer={
          <>
            <Button size={36} variant="ghost">
              Cancelar
            </Button>
            <Button size={36} variant="accent">
              Guardar
            </Button>
          </>
        }
        onClose={close}
        subtitle="Solo cambia esta nota."
        title="Cambiar el título"
      >
        <Field defaultValue="Revisión del diseño de reservas" label="Título" />
      </Dialog>
    </div>
  );
}

// Always open in the sheet, so the captures, whose presses land outside it, still show it.
const keepOpen = () => undefined;

function SheetPopover() {
  return (
    <div className={styles.frame} style={{ width: 300, height: 380, flex: "none", padding: 16 }}>
      <Popover label="Cambiar quién habla" onOpenChange={keepOpen} open trigger="Marta Ibáñez ▾">
        <div className={styles.stack} style={{ gap: 2 }}>
          {["Marta Ibáñez", "Diego Salinas", "Javier Ortega"].map((person) => (
            <Button key={person} size={32} variant="ghost">
              {person}
            </Button>
          ))}
        </div>
      </Popover>
    </div>
  );
}
