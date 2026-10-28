import { hubAdmissionRefusals } from "./hubAdmissionRefusals";
import { hubRequestRefusals } from "./hubRequestRefusals";
import { dynamicHubRefusal } from "./hubDynamicRefusals";
import type { Numbers } from "./refusalNumbers";

const messages: Record<string, (n: Numbers) => string | undefined> = {
  ...hubAdmissionRefusals,
  ...hubRequestRefusals,
};

export function hubRefusalText(code: string, numbers: Numbers): string | undefined {
  return (
    dynamicHubRefusal(code, numbers) ??
    (Object.hasOwn(messages, code) ? messages[code]?.(numbers) : undefined)
  );
}
