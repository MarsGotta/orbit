---
type: proyecto
title: Piloto Hoteles Ribera
description: Cinco hoteles de Hoteles Ribera prueban el nuevo flujo de reserva antes de llevarlo a toda la cadena.
tags: [cliente, piloto]
estado: en marcha
responsable: /personas/lucia-ferrer.md
cliente: /organizaciones/hoteles-ribera.md
arranque: 2026-10-26
status: stable
sources:
  - id: r1021
    resource: /fuentes/reuniones/2026-10-21-revision-del-diseno-de-reservas.md
    title: Revisión del diseño de reservas · 21 oct
  - id: r1014
    resource: /fuentes/reuniones/2026-10-14-revision-del-diseno-de-reservas.md
    title: Revisión del diseño de reservas · 14 oct
  - id: r1009
    resource: /fuentes/reuniones/2026-10-09-hoteles-ribera-arranque-del-piloto.md
    title: Hoteles Ribera · arranque del piloto · 9 oct
generated:
  by: deimos/llm-wiki-2.1
  at: 2026-10-21T11:52:00+02:00
verified:
  - by: human:lucia
    at: 2026-10-21T12:05:00+02:00
stale_after: 2026-11-21T00:00:00+01:00
---
> **En marcha** · La caché de tarifas por día ya funciona y Ribera Aeropuerto puede hacer la demo. Falta el sí de [Pablo Medina](/personas/pablo-medina.md) a la fecha de arranque. [^r1021]
## Dependencias
- El **arranque** (propiedad `arranque`) depende de [Preparar la demo con Ribera Aeropuerto](#^demo-aeropuerto) y de [Confirmar con Pablo la fecha de arranque](#^si-de-pablo).
- [Preparar la demo con Ribera Aeropuerto](#^demo-aeropuerto) dependía de [Cachear las tarifas por día](#^cache-tarifas), ya hecha.
- [Preparar la formación de Ribera Playa](#^formacion-playa) va después del arranque: no lo bloquea.
## Tareas
- [ ] Confirmar con Pablo la fecha de arranque · [Lucía Ferrer](/personas/lucia-ferrer.md) · para el 2026-10-23 [^r1014] ^si-de-pablo
- [x] Cachear las tarifas por día · [Javier Ortega](/personas/javier-ortega.md) · hecha el 2026-10-21 [^r1021] ^cache-tarifas
- [ ] Preparar la demo con Ribera Aeropuerto · [Diego Salinas](/personas/diego-salinas.md) · para el 2026-10-23 [^r1021] ^demo-aeropuerto
- [ ] Preparar la formación de Ribera Playa · [Sara Llorente](/personas/sara-llorente.md) · para el 2026-10-27 [^r1009] ^formacion-playa
- [ ] Enviar el resumen al equipo de Ribera · [Lucía Ferrer](/personas/lucia-ferrer.md) [^r1014]
## Hoteles
- **Ribera Centro** · 84 habitaciones · formación hecha
- **Ribera Playa** · 120 habitaciones · formación el martes 27
- **Ribera Golf** · 46 habitaciones · formación hecha
- **Casa Ribera** · 18 habitaciones · formación hecha
- **Ribera Aeropuerto** · 210 habitaciones · demo el jueves 23 [^r1021]
## Decisiones
- [Cinco hoteles en el piloto](/decisiones/cinco-hoteles-en-el-piloto.md) · 9 oct
- [Pago en una sola pantalla](/decisiones/pago-en-una-sola-pantalla.md) · 14 oct
- [El bloqueo para grupos, después del piloto](/decisiones/el-bloqueo-para-grupos-despues-del-piloto.md) · 14 oct

[^r1021]: [Revisión del diseño de reservas · 21 oct](/fuentes/reuniones/2026-10-21-revision-del-diseno-de-reservas.md)
[^r1014]: [Revisión del diseño de reservas · 14 oct](/fuentes/reuniones/2026-10-14-revision-del-diseno-de-reservas.md)
[^r1009]: [Hoteles Ribera · arranque del piloto · 9 oct](/fuentes/reuniones/2026-10-09-hoteles-ribera-arranque-del-piloto.md)
