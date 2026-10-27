---
type: persona
title: Javier Ortega
description: Backend de reservas en Nubaria. Lleva la caché de tarifas del piloto de Hoteles Ribera.
tags: [equipo, backend]
rol: Backend de reservas
organizacion: /organizaciones/nubaria.md
correo: javier.ortega@nubaria.example
status: stable
sources:
  - id: r1021
    resource: /fuentes/reuniones/2026-10-21-revision-del-diseno-de-reservas.md
    title: Revisión del diseño de reservas · 21 oct
  - id: r1020
    resource: /fuentes/reuniones/2026-10-20-llamada-con-javier-sobre-la-api-de-tarifas.md
    title: Llamada con Javier sobre la API de tarifas · 20 oct
  - id: r1014
    resource: /fuentes/reuniones/2026-10-14-revision-del-diseno-de-reservas.md
    title: Revisión del diseño de reservas · 14 oct
generated:
  by: deimos/llm-wiki-2.1
  at: 2026-10-21T11:52:00+02:00
verified:
  - by: human:lucia
    at: 2026-10-21T12:05:00+02:00
stale_after: 2026-11-21T00:00:00+01:00
---
## Quién es
Lleva el backend de reservas en [Nubaria](/organizaciones/nubaria.md). Trabaja con [Marta Ibáñez](/personas/marta-ibanez.md) en el [Flujo de reserva en una pantalla](/proyectos/flujo-de-reserva-en-una-pantalla.md).
## En qué está
- Terminó la caché de tarifas por día: la disponibilidad baja a unos 300 ms con cinco hoteles. [^r1021]
- La API de tarifas ya devuelve precios por día. [^r1020]
## Tareas
```tareas
persona: Javier Ortega
```
## Lo último
- **21 oct** · En la [Revisión del diseño de reservas](/fuentes/reuniones/2026-10-21-revision-del-diseno-de-reservas.md): con la caché, unos 300 ms con cinco hoteles. [^r1021]
- **20 oct** · Llamada sobre la API de tarifas: ya devuelve precios por día. [^r1020]
- **14 oct** · Propuso cachear las tarifas por día: con cincuenta hoteles la disponibilidad no aguantaría. [^r1014]
## Cómo trabajar con él
Prefiere los cambios por escrito antes de la daily.

[^r1021]: [Revisión del diseño de reservas · 21 oct](/fuentes/reuniones/2026-10-21-revision-del-diseno-de-reservas.md)
[^r1020]: [Llamada con Javier sobre la API de tarifas · 20 oct](/fuentes/reuniones/2026-10-20-llamada-con-javier-sobre-la-api-de-tarifas.md)
[^r1014]: [Revisión del diseño de reservas · 14 oct](/fuentes/reuniones/2026-10-14-revision-del-diseno-de-reservas.md)
