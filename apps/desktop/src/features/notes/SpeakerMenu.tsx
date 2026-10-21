import { useState } from "react";
import type { OpenNoteSnapshot } from "../../bridge/generated/OpenNoteSnapshot";
import { Avatar } from "../../design/Avatar";
import { CommandPalette, type CommandItem } from "../../design/CommandPalette";
import { es } from "../../i18n/es";

interface SpeakerMenuProps {
  note: OpenNoteSnapshot;
  speakerKey: string;
  onChoose: (name: string) => void;
  onAdd: (name: string) => void;
  onClose: () => void;
}

function person(name: string) {
  return <Avatar initial={name} label={name} />;
}

/** Guests suggest names; only the hub decides what a rename changes. */
export function SpeakerMenuView({ note, speakerKey, onChoose, onAdd, onClose }: SpeakerMenuProps) {
  const [query, setQuery] = useState("");
  const guests: CommandItem[] = (note.event?.attendees ?? [])
    .filter((guest) => !guest.is_self)
    .map((guest, index) => ({
      id: `guest-${index}`,
      icon: "calendar",
      label: guest.name || guest.email,
      subtitle: guest.email,
      group: es.notes.guests,
      leading: person(guest.name || guest.email),
      onSelect: () => onChoose(guest.name || guest.email),
    }));
  const speakers: CommandItem[] = note.speakers
    .filter((speaker) => {
      const voice = /^voice_(\d+)$/.exec(speaker.key);
      return (
        speaker.name !== "Otra persona" &&
        (voice === null || speaker.name !== `Persona ${voice[1]}`)
      );
    })
    .map((speaker) => ({
      id: speaker.key,
      icon: "mic",
      label: speaker.name,
      subtitle: "",
      group: es.notes.recordingSpeakers,
      leading: person(speaker.name),
      checked: speaker.key === speakerKey,
      onSelect: () => onChoose(speaker.name),
    }));
  const search = query.trim().toLocaleLowerCase("es");
  const commands = [...guests, ...speakers].filter((item) =>
    `${item.label} ${item.subtitle}`.toLocaleLowerCase("es").includes(search),
  );
  commands.push({
    id: "add",
    icon: "plus",
    label: es.notes.addSpeaker,
    subtitle: query.trim(),
    onSelect: () => onAdd(query.trim()),
  });
  return (
    <CommandPalette
      commands={commands}
      open
      title={es.notes.changeSpeaker}
      inputLabel={es.notes.speakerSearch}
      query={query}
      onQueryChange={setQuery}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    />
  );
}
