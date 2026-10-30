import { useCopy } from "../../i18n/CopyProvider";
import { TopBarLeading } from "../../app/shell-slots";
import type { VaultRoute } from "./route";
export function VaultPageBar({ route }: { route: VaultRoute }) {
  const copy = useCopy().extras.vaultCopy;
  if (route.section === "note" || route.section === "distill") return null;
  return (
    <TopBarLeading>
      <span>
        {route.section === "tasks" ? copy.tasks : route.section === "map" ? copy.map : copy.home}
      </span>
    </TopBarLeading>
  );
}
