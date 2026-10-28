import { useCopy } from "../../i18n/CopyProvider";
import { Switch } from "../../design/Switch";

import { Group, List, Row } from "../settings/layout";
export function LaterSettings() {
  const copy = useCopy().extras.vaultCopy;
  const switches = [
    ["Fichas de personas", true],
    ["Proyectos y dependencias", true],
    ["Tareas", true],
    ["Decisiones", true],
    ["Usar también mis notas privadas", false],
    ["Guardar la transcripción entera en la fuente", true],
  ] as const;
  return (
    <>
      <Group help={copy.phaseTwo} label="Destilar reuniones">
        <List>
          <Row
            help="Te enseña los cambios nota a nota, con lo que se dijo. Aplicas todo, una parte o lo editas. Las notas quedan como revisadas por ti."
            title="Revisar antes de aplicar"
          />
          <Row
            help="Los aplica al terminar la nota. Te avisa en la reunión, quedan en el registro y se deshacen con un clic. Las notas quedan sin revisar."
            title="Aplicar solo y avisarme"
          >
            <Switch
              checked={false}
              disabled
              label="Aplicar solo y avisarme"
              onCheckedChange={() => undefined}
            />
          </Row>
          {switches.map(([label, checked]) => (
            <Row
              aside={
                <Switch
                  checked={checked}
                  disabled
                  label={label}
                  onCheckedChange={() => undefined}
                />
              }
              key={label}
              title={label}
            />
          ))}
        </List>
      </Group>
      <ReviewDefaults />
    </>
  );
}

function ReviewDefaults() {
  const copy = useCopy().extras.vaultCopy;
  return (
    <Group help={copy.phaseTwo} label="Revisión del vault">
      <List>
        <Row
          help="Busca contradicciones, fichas caducadas, notas sin enlaces, enlaces rotos y lo que no cumple OKF."
          title="Cuándo"
        >
          <select aria-label="Cuándo se revisa" disabled value="daily">
            <option value="daily">Cada día a las 9:00</option>
          </select>
        </Row>
        <Row
          help="Pasado ese tiempo sin revisarla, aparece en Revisar (stale_after)."
          title="Una ficha caduca a los"
        >
          <select aria-label="Caducidad" disabled value="30">
            <option value="30">30 días</option>
          </select>
        </Row>
      </List>
    </Group>
  );
}
