---
type: guide
title: Vault maintenance method
---
# Markdown vault method

This is a template for a vault's agent instructions. Orbit currently reads
notes; it does not run ingestion, maintenance or writes.

Read `SCHEMA.md`, root `index.md` and `log.md` before proposing a change.
Keep source transcripts and the person's own prose unchanged. Propose a
conflict when a new source contradicts existing material. A future hub job
with its own configured model will prepare proposals.

`SCHEMA.md` frontmatter defines this vault's type names and field aliases:

```yaml
---
type: schema
types:
  person: {role: person, folder: People/}
  project: {role: project, folder: Projects/}
  client: {role: project, folder: Clients/}
  meeting: {role: meeting, folder: Meetings/}
  topic: {role: topic}
  source: {role: source, folder: Sources/}
  journal: {role: journal, folder: Journal/}
fields:
  stage: phase
  start: kickoff
---
Free prose about this vault's conventions.
```

The roles are `person`, `project`, `meeting`, `topic`, `source`, `journal`
and `note`. Multiple types can have the same role. A note without `type`
inherits the type of its longest matching declared folder. An explicit,
undeclared type keeps its name and has role `note`. Without `types`, the
six types other than `note` default to their matching roles; there are no
implicit folder mappings. Without a type or matching folder, a note has type
and role `note`. The body of `SCHEMA.md` is free text, never configuration.

`fields` may rename only `stage` (the project's progress) and `start` (its
kickoff). Omitted mappings use those names. OKF fields keep their standard
names and cannot be targets of these aliases. Invalid schema YAML,
roles, folders or field mappings produce a Spanish reading notice and use
all defaults. Notes preserve unknown properties, including extensions inside
sources and actor events, and their original Markdown. A future writer must
round-trip the complete frontmatter, never replace it with parsed properties.

Every concept needs a non-empty `type`, including `SCHEMA.md` (`type: schema`)
and this guide. Type names are free. A missing type is still read, including
folder inference, but the note's conformance data records the missing field.
Conformance issues are advisory data for a future review, without new UI notices.

Use OKF 0.2 provenance: `sources` entries carry `id`, `resource` and `title`;
`generated` is an actor/time map; `generated.at` records the last meaningful
content change. `verified` is a list of actor/time maps; a single map is read
as a one-element list. The timestamps in `generated.at`, `verified[].at` and
`stale_after` must be ISO 8601 datetimes with explicit UTC offsets (`Z` or
`+02:00`). Actors use `producer/version`, `human:id` or `process:id`.

`status` is the note's lifecycle: `draft`, `stable` or `deprecated`, with
`stable` as the absent value. The project's `stage` is independent of it.
Invalid lifecycle, actors or timestamps are reported softly on the note;
they never prevent reading it.
A missing verification is not a human sign-off. Aim for about 400 words per
fiche; longer fiches raise a review warning. Source transcripts are exempt.
Use lowercase, hyphen-separated file names and standard Markdown links with
absolute bundle paths, for example `[Ada Vale](/People/ada-vale.md)`. This is
the default writing style. Orbit also reads Obsidian `[[wikilinks]]`.
Cite claims with `[^id]` and a matching `sources[].id`.

Store a task once, normally in its project:

```markdown
- [ ] Ask [Ada Vale](/People/ada-vale.md) about the garden #garden 📅 2026-10-30 ⏳ 2026-10-29 🛫 2026-10-28 ⏫ 🔁 every week 🆔 ask
- [x] Prepare the plan ✅ 2026-10-27 🆔 prepare ^plan
- [ ] Plant the garden [Ada Vale](/People/ada-vale.md) 🆔 plant ⛔ ask,prepare
```

The first linked note with role `person` owns the task. Priority markers
are `⏫`, `🔼` and `🔽`; recurrence is shown raw. Tasks ids (`🆔`) resolve
across the vault; `⛔` lists prerequisite ids. An undated dependent task in a
project follows its kickoff (`start`, renamed to `kickoff` above). Obsidian
block ids (`^id`) remain independent of Tasks ids.

A `tasks` code block supports `done`, `not done`, `due before|after|on
YYYY-MM-DD`, `path includes text`, `description includes text` and `tags
include #tag`. All understood lines apply together. Each unsupported line
produces a Spanish notice inside the view. Query views show at most 100
matches and report the total if more exist. They never duplicate task records.

Only `index.md` and `log.md` are reserved, at any directory level. The root
`index.md` may carry frontmatter with `okf_version: "0.2"`; nested indexes
have no frontmatter. Index bodies contain headed lists:

```markdown
# People
* [Ada Vale](/People/ada-vale.md) - A gardener.
```

`log.md` has no frontmatter. It lists updates newest first under `## YYYY-MM-DD`
headings. Entries are prose; Orbit's own form remains
`- HH:MM · action · [Title](/path.md) · by · detail`. Other entries remain
visible as plain text. Future accepted writes update the index and append
entries to the latest date group, preserving the newest-first order.
Never silently choose between contradictory sources.
