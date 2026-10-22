import { useRef } from "react";
import type { NoteTemplateBoundsSnapshot } from "../../../bridge/generated/NoteTemplateBoundsSnapshot";
import type { TemplateSection } from "../../../bridge/generated/TemplateSection";
import { Button } from "../../../design/Button";
import { Icon } from "../../../design/Icon";
import { es } from "../../../i18n/es";
import { moveSection } from "./template-draft";
import styles from "./Templates.module.css";

type Sections = TemplateSection[];

/** The handle moves its section with the mouse, or with ↑ and ↓ from the keyboard. */
function SectionHandle({ name, onMove }: { name: string; onMove: (by: -1 | 1) => void }) {
  return (
    <button
      aria-label={es.library.dragSection(name)}
      className={styles.handle}
      onKeyDown={(event) => {
        if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
        event.preventDefault();
        onMove(event.key === "ArrowUp" ? -1 : 1);
      }}
      type="button"
    >
      ⋮⋮
    </button>
  );
}

/** «Añadir sección», up to the hub's ceiling; there, why not. */
function AddSection({ count, max, onAdd }: { count: number; max: number; onAdd: () => void }) {
  return (
    <Button
      className={styles.add}
      disabledReason={count >= max ? es.library.sectionsFull(max) : undefined}
      onClick={onAdd}
      size={28}
      variant="ghost"
    >
      <Icon name="plus" size={14} />
      {es.library.addSection}
    </Button>
  );
}

function SectionRow(props: {
  bounds: NoteTemplateBoundsSnapshot;
  index: number;
  onChange: (section: TemplateSection) => void;
  onDragStart: () => void;
  onDrop: () => void;
  onMove: (by: -1 | 1) => void;
  onRemove?: () => void;
  section: TemplateSection;
}) {
  const { bounds, index, section } = props;
  return (
    <li
      className={styles.section}
      draggable
      onDragOver={(event) => event.preventDefault()}
      onDragStart={props.onDragStart}
      onDrop={(event) => {
        event.preventDefault();
        props.onDrop();
      }}
    >
      <SectionHandle name={section.name} onMove={props.onMove} />
      <div className={styles.sectionFields}>
        <input
          aria-label={es.library.sectionName(index)}
          className={styles.sectionName}
          maxLength={bounds.max_name_chars}
          onChange={(event) => props.onChange({ ...section, name: event.target.value })}
          value={section.name}
        />
        <textarea
          aria-label={es.library.sectionInstructions(index)}
          className={styles.sectionHelp}
          maxLength={bounds.max_instructions_chars}
          onChange={(event) => props.onChange({ ...section, instructions: event.target.value })}
          rows={1}
          value={section.instructions}
        />
      </div>
      {props.onRemove && (
        <button
          aria-label={es.library.removeSection(section.name)}
          className={styles.remove}
          onClick={props.onRemove}
          type="button"
        >
          <Icon name="close" size={14} />
        </button>
      )}
    </li>
  );
}

/**
 * The ordered sections, «3 de 12»: rename, instruct, reorder by dragging or
 * from the keyboard, add and remove (one always stays).
 */
export function TemplateSections({
  bounds,
  onChange,
  sections,
}: {
  bounds: NoteTemplateBoundsSnapshot;
  onChange: (sections: Sections) => void;
  sections: Sections;
}) {
  const dragged = useRef<number | null>(null);
  const list = useRef<HTMLOListElement>(null);
  const replace = (index: number, section: TemplateSection) =>
    onChange(sections.map((current, at) => (at === index ? section : current)));
  const move = (from: number, to: number) => {
    if (to < 0 || to >= sections.length) return;
    onChange(moveSection(sections, from, to));
    // The handle follows its section, so ↑ ↓ can keep moving it.
    requestAnimationFrame(() =>
      list.current?.querySelectorAll<HTMLButtonElement>(`.${styles.handle}`)[to]?.focus(),
    );
  };
  return (
    <div className={styles.sections}>
      <div className={styles.sectionsLabel}>
        <span>{es.library.sections}</span>
        <span>{es.library.sectionCount(sections.length, bounds.max_sections)}</span>
      </div>
      <ol ref={list}>
        {sections.map((section, index) => (
          <SectionRow
            bounds={bounds}
            index={index}
            key={index}
            onChange={(next) => replace(index, next)}
            onDragStart={() => {
              dragged.current = index;
            }}
            onDrop={() => {
              if (dragged.current !== null) onChange(moveSection(sections, dragged.current, index));
              dragged.current = null;
            }}
            onMove={(by) => move(index, index + by)}
            onRemove={
              sections.length > 1
                ? () => onChange(sections.filter((_, at) => at !== index))
                : undefined
            }
            section={section}
          />
        ))}
      </ol>
      <AddSection
        count={sections.length}
        max={bounds.max_sections}
        onAdd={() => onChange([...sections, { name: es.library.newSectionName, instructions: "" }])}
      />
    </div>
  );
}
