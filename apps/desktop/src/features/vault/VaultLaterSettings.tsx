import { useCopy } from "../../i18n/CopyProvider";
import { Switch } from "../../design/Switch";

import { Group, List, Row } from "../settings/layout";
export function LaterSettings() {
  const inlineCopy = useCopy().vaultSettings;
  const copy = useCopy().extras.vaultCopy;
  const switches = [
    [inlineCopy.people, true],
    [inlineCopy.projects, true],
    [inlineCopy.tasks, true],
    [inlineCopy.decisions, true],
    [inlineCopy.privateNotes, false],
    [inlineCopy.fullTranscript, true],
  ] as const;
  return (
    <>
      <Group help={copy.phaseTwo} label={inlineCopy.distill}>
        <List>
          <Row help={inlineCopy.reviewHelp} title={inlineCopy.reviewFirst} />
          <Row help={inlineCopy.autoApplyHelp} title={inlineCopy.autoApply}>
            <Switch
              checked={false}
              disabled
              label={inlineCopy.autoApply}
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
  const inlineCopy = useCopy().vaultSettings;
  const copy = useCopy().extras.vaultCopy;
  return (
    <Group help={copy.phaseTwo} label={inlineCopy.review}>
      <List>
        <Row help={inlineCopy.reviewChecks} title={inlineCopy.when}>
          <select aria-label={inlineCopy.reviewSchedule} disabled value="daily">
            <option value="daily">{inlineCopy.daily}</option>
          </select>
        </Row>
        <Row help={inlineCopy.staleHelp} title={inlineCopy.expiresAfter}>
          <select aria-label={inlineCopy.expiry} disabled value="30">
            <option value="30">{inlineCopy.thirtyDays}</option>
          </select>
        </Row>
      </List>
    </Group>
  );
}
