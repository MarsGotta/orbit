# Markdown vault method

This is a template for a vault's agent instructions. Orbit currently reads
the vault; it does not run ingestion, maintenance or writes.

Read `SCHEMA.md`, root `index.md` and `log.md` before proposing a change.
`SCHEMA.md` is the authority for this vault's templates, sections and rules.
Keep source material under `fuentes/` unchanged. Keep the person's own prose;
propose a conflict when a new source contradicts it. A hub job with its own
configured model will prepare proposals in a later phase.

Use these types and folders: `persona` → `personas/`, `proyecto` →
`proyectos/`, `organizacion` → `organizaciones/`, `decision` →
`decisiones/`, `tema` → `temas/`, `consulta` → `consultas/`, `fuente` →
`fuentes/`. A loose `nota` may live in any folder. File names are lowercase,
unaccented titles separated by hyphens. Meeting sources use
`fuentes/reuniones/YYYY-MM-DD-title.md`; documents use `fuentes/documentos/`.

Use OKF 0.2 YAML: `type`, `title`, `description`, `tags`, `status`,
`sources`, `generated`, `verified`, `stale_after`. `sources` entries carry
`id`, `resource` and `title`. `generated` is an actor/time map; `verified`
is a list of independent actor/time maps. A missing verification is not a
human sign-off. Preserve unknown fields when reading existing notes.
People may also have `rol`, `organizacion`, `correo`; projects `estado`,
`responsable`, `cliente`, `arranque`; sources `resource`, `fecha`,
`duracion_min`, `serie`, `participantes`, `sha256`.

Aim for about 400 words per fiche. A longer fiche raises a review warning:
propose splitting it into linked notes. Source transcripts are exempt.
Use Markdown links relative to the vault root by default, for example
`[Garden](/proyectos/garden.md)`, or wikilinks when the link-style setting
selects them. Cite source-derived claims with `[^id]` and a matching
`sources[].id` and footnote definition.

Store a task once, in its project (or its person's fiche without a project):
`- [ ] Task · [Person](/personas/ada.md) · para el 2026-10-30 [^source] ^task-id`.
Completed tasks use `[x]` and `hecha el YYYY-MM-DD`. Dependencies link to
tasks with `#^task-id`. The `tareas` and `dependencias` fenced query blocks
are reading views, not duplicate task records.

Ingestion proposes changes from a meeting or document. Query cites only
what the vault supports. Review proposes remedies for contradictions,
staleness, broken links, missing metadata and long fiches. None runs in this
phase. Future accepted writes update root `index.md` (`okf_version: "0.2"`)
and append to `log.md`: `- HH:MM · action · [title](path) · actor · detail`.
Never silently choose between contradictory sources.
