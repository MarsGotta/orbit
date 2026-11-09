import { useCopy } from "../../i18n/CopyProvider";
import type { VaultDiagnostic } from "../../bridge/generated/VaultDiagnostic";

/** Older hubs supply prose only; current hubs also supply stable diagnostics. */
export function VaultWarnings({
  diagnostics,
  warnings,
  className,
}: {
  diagnostics?: VaultDiagnostic[];
  warnings: string[];
  className?: string;
}) {
  const copy = useCopy();
  const entries =
    diagnostics !== undefined && diagnostics.length > 0
      ? diagnostics
      : warnings.map(() => ({
          code: "legacy_vault_warning",
          field: "",
          numbers: {},
        }));
  return (
    <>
      {entries.map((diagnostic, index) => (
        <p role="alert" className={className} key={`${diagnostic.code}:${index}`}>
          {copy.extras.vaultWarningText(diagnostic)}
        </p>
      ))}
    </>
  );
}
