import { useEffect, useRef, useState } from "react";
import type { AppCommand } from "../../bridge/generated";
import { Button } from "../../design/Button";
import { es } from "../../i18n/es";

export function NotesRefreshButton({ send }: { send: (command: AppCommand) => Promise<void> }) {
  const [refreshing, setRefreshing] = useState(false);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const refresh = async () => {
    setRefreshing(true);
    try {
      await send({ command: "notes_refresh" });
    } finally {
      if (mounted.current) setRefreshing(false);
    }
  };
  return (
    <Button disabled={refreshing} onClick={() => void refresh()}>
      {refreshing ? es.notes.refreshing : es.notes.refresh}
    </Button>
  );
}
