import { useRef } from "react";
import type { NoteTemplateBoundsSnapshot } from "../../../bridge/generated/NoteTemplateBoundsSnapshot";
import type { TemplateSection } from "../../../bridge/generated/TemplateSection";
import { Button } from "../../../design/Button";
import { es } from "../../../i18n/es";
import { moveSection } from "./template-draft";
import styles from "./Templates.module.css";

type Sections = TemplateSection[];

function SectionRow(props: {
  bounds: NoteTemplateBoundsSnapshot;
  index: number;
  onChange: (section: TemplateSection) => void;
  onDragStart: () => void;
  onDrop: () => void;
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
      <span aria-label={es.library.dragSection(section.name)} className={styles.handle} role="img">
        ⋮⋮
      </span>
      <div className={styles.sectionFields}>
        <input
          aria-label={es.library.sectionName(index)}
          maxLength={bounds.max_name_chars}
          onChange={(event) => props.onChange({ ...section, name: event.target.value })}
          value={section.name}
        />
        <textarea
          aria-label={es.library.sectionInstructions(index)}
          maxLength={bounds.max_instructions_chars}
          onChange={(event) => props.onChange({ ...section, instructions: event.target.value })}
          rows={2}
          value={section.instructions}
        />
      </div>
      {props.onRemove && (
        <Button
          aria-label={es.library.removeSection(section.name)}
          onClick={props.onRemove}
          size={28}
          variant="ghost"
        >
          ×
        </Button>
      )}
    </li>
  );
}

/** The ordered sections: rename, instruct, drag to reorder, add and remove. */
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
  const replace = (index: number, section: TemplateSection) =>
    onChange(sections.map((current, at) => (at === index ? section : current)));
  return (
    <div className={styles.sections}>
      <p className={styles.hint}>{es.library.sections}</p>
      <ol>
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
            onRemove={
              sections.length > 1
                ? () => onChange(sections.filter((_, at) => at !== index))
                : undefined
            }
            section={section}
          />
        ))}
      </ol>
      <Button
        disabled={sections.length >= bounds.max_sections}
        onClick={() =>
          onChange([...sections, { name: es.library.newSectionName, instructions: "" }])
        }
        size={28}
        variant="outline"
      >
        {es.library.addSection}
      </Button>
    </div>
  );
}
