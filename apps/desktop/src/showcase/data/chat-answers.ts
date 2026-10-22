/** Deimos's answers in the invented conversation, in the Markdown it writes. */

export const prepAnswer = `## Para la llamada de las 16:00 con Hoteles Ribera

Repasé la nota del **arranque del piloto** (9 de octubre) y la revisión de diseño de la semana pasada. Esto es lo que conviene llevar:

### Lo que quedó pendiente con ellos
- Confirmar la **fecha de arranque**: dijisteis el lunes 26, falta su sí.
- Sara Llorente será la persona de contacto en los hoteles.
- Pidieron poder **bloquear habitaciones para grupos**; lo dejasteis para después del piloto.

### Estado de los cinco hoteles

| Hotel | Habitaciones | Formación | Estado |
| --- | ---: | --- | --- |
| Ribera Centro | 84 | Hecha | Listo |
| Ribera Playa | 120 | Martes 27 | Pendiente |
| Ribera Golf | 46 | Hecha | Listo |
| Casa Ribera | 18 | Hecha | Listo |
| Ribera Aeropuerto | 210 | Sin fecha | Bloqueado |

> Ribera Aeropuerto depende de la caché de tarifas de Javier: sin ella, la disponibilidad tarda más de 2 s.

Si quieres, preparo un correo para Pablo con la fecha y la formación pendiente.`;

export const queryAnswer = `Claro. Esta consulta da la **ocupación por hotel y día** de la semana del piloto, lista para pegar en el informe:

\`\`\`sql
select h.name as hotel,
       d.day::date as dia,
       count(b.id) filter (where b.status = 'confirmed') as reservas,
       round(100.0 * count(b.id) / h.rooms, 1) as ocupacion
from hotels h
cross join generate_series('2026-10-26'::date, '2026-11-01', '1 day') as d(day)
left join bookings b
  on b.hotel_id = h.id and d.day between b.check_in and b.check_out - 1
where h.chain = 'ribera'
group by h.name, h.rooms, d.day
order by h.name, d.day;
\`\`\`

Dos detalles:

1. \`check_out - 1\` cuenta la última noche, no el día de salida.
2. Si algún hotel no tiene reservas un día, sale con **0** gracias al \`left join\`.`;

export const mailAnswer = `Aquí tienes el borrador. No lo he enviado:

---

**Asunto:** Arranque del piloto el lunes 26

Hola, Pablo:

Te confirmo que empezamos el **lunes 26 de octubre** con los cinco hoteles. Nos queda la formación de *Ribera Playa*, que proponemos para el martes 27 a las 10:00.

Ribera Aeropuerto entra en cuanto terminemos una mejora de velocidad, previsiblemente el jueves.

Un saludo,
Lucía

---

¿Lo dejo en Borradores de Mail o prefieres cambiar algo antes?`;
