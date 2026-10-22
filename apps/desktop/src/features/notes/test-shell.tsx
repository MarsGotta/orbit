import type { ReactNode } from "react";
import { ShellSlots, useShellSlots } from "../../app/shell-slots";

function ColumnHost() {
  return <div ref={useShellSlots()?.setColumn} />;
}

/** The window's list column, for a test that renders what fills it. */
export function WithListColumn({ children }: { children: ReactNode }) {
  return (
    <ShellSlots>
      <ColumnHost />
      {children}
    </ShellSlots>
  );
}
