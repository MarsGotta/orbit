---
type: esquema
title: Esquema del vault
description: Cómo se organiza este vault y las reglas que sigue el asistente que lo mantiene.
status: stable
---
## Para qué es
Las reglas que sigue Deimos al escribir en este vault. Las lee antes de cada cambio, junto con el índice y el registro.
## Carpetas
- `personas/` · `type: persona` · una por persona
- `proyectos/` · `type: proyecto` · estado, dependencias, tareas y decisiones
- `organizaciones/` · `type: organizacion`
- `decisiones/` · `type: decision` · qué, por qué y qué se descartó
- `temas/` · `type: tema` · lo que sabemos y lo que falta
- `consultas/` · `type: consulta` · respuestas que merecían quedarse
- `fuentes/` · reuniones y documentos tal cual: **no se cambian nunca**
## Reglas
1. Cada frase nueva lleva su fuente: `[^id]` en el texto y la fuente en `sources`.
2. Si algo nuevo contradice lo que hay, no se elige: se le propone a Lucía.
3. Lo que Lucía escribe a mano no se reescribe sin preguntar.
4. Cada tarea se escribe una vez, en su proyecto: `- [ ] tarea · [persona](/personas/…) · para el AAAA-MM-DD [^fuente] ^id`.
5. Las dependencias van en la tarea o en el proyecto: **depende de** [otra nota](/ruta.md) o de otra tarea del proyecto por su `^id`.
6. Al terminar, se actualiza `index.md` y se añade una línea a `log.md`.
## Destilar una reunión
- Leer la nota de la reunión y su transcripción.
- Guardarla en `fuentes/reuniones/` con su `sha256`.
- Actualizar las fichas de quienes hablaron y los proyectos y temas que salieron; crear las decisiones nuevas.
- Proponer los cambios o aplicarlos, según Ajustes › Notas.
