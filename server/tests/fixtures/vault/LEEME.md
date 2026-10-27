# Vault de ejemplo

Los mismos datos del prototipo (Nubaria, el piloto con Hoteles Ribera), con el destilado de la «Revisión del diseño de reservas» del 21 oct ya aplicado. Sirve para ver el formato de cada archivo y para probar el lector, el editor y el destilado con algo real.

Solo trae una parte del vault de 26 notas que se ve en el prototipo: el esquema, el índice, el registro, tres personas, el proyecto, una consulta y una fuente. Los enlaces a notas que no están aquí apuntan a su ruta, igual que en el vault entero.

- **Enlaces.** Van como enlace Markdown con ruta (`[Nubaria](/organizaciones/nubaria.md)`), que es lo que pide OKF y la opción por defecto en Ajustes › Notas. Con «[[Wikilinks]]» se escribirían `[[Nubaria]]`.
- **Citas.** Cada frase que sale de una reunión lleva `[^id]`. El `id` es el mismo que en `sources` y la definición va al final del archivo.
- **Tareas.** Se escriben una vez, en la nota de su proyecto: `- [ ] texto · [persona](/personas/…) · para el AAAA-MM-DD [^fuente] ^id`. Las fichas de persona las juntan con un bloque `tareas`.
- **Dependencias.** Enlazan tareas por su `^id` dentro del proyecto.
- **Fuentes.** Las reuniones van a `fuentes/` tal cual y no se cambian nunca. El `sha256` es del cuerpo del archivo.
