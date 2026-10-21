import type { HelperCode } from "../../bridge/generated/HelperCode";
import type { Refusal } from "../../bridge/refusal";

const helperMessages: Record<HelperCode, string> = {
  accessibility_permission_required:
    "macOS no concede a Orbit el permiso de Accesibilidad en ese Mac, y sin él no puede usar el ordenador. Se concede en Ajustes del Sistema › Privacidad y seguridad › Accesibilidad.",
  calendar_busy:
    "EventKit seguía terminando una lectura anterior de la agenda en ese Mac y Orbit no la interrumpe. Vuelve a preguntar en unos segundos.",
  calendar_clock_unavailable:
    "Orbit no pudo verificar el reloj de esa lectura de la agenda. Esto NO significa que la agenda esté vacía.",
  calendar_event_too_large:
    "Una página de la agenda del Mac pasa del mayor mensaje que su lector puede entregar aun dejando fuera, con su nombre, los eventos que no caben, porque la lista de esos eventos es demasiado larga. Nada se recortó. Esto NO significa que la agenda esté vacía; pregunta por un rango más corto.",
  calendar_invalidated:
    "El calendario cambió durante la lectura. Vuelve a pedir la agenda; su contenido sigue sin conocerse.",
  calendar_limit_exceeded:
    "Un evento o calendario tiene un campo más largo de lo que Orbit lee, y la lectura se detuvo en vez de recortarlo. Esto NO significa que la agenda esté vacía.",
  calendar_permission_required:
    "Orbit no tiene acceso de lectura completo a tus Calendarios en macOS, así que no puede leer la agenda. Esto NO significa que la agenda esté vacía. Concédelo en Ajustes del Sistema › Privacidad y seguridad › Calendarios.",
  calendar_read_expired:
    "EventKit no terminó de leer la agenda en el plazo de una lectura, ni siquiera en el tramo más corto que Orbit le pide. Vuelve a preguntar en unos segundos.",
  calendar_read_failed:
    "EventKit devolvió un error al leer la agenda en el Mac. Esto NO significa que la agenda esté vacía.",
  calendar_selection_unavailable:
    "El calendario pedido no está disponible para esta lectura. Su contenido sigue sin conocerse.",
  capture_clock_unavailable:
    "El helper no pudo comprobar el reloj de la captura. No se entregó ninguna lectura; vuelve a pedirla.",
  invalid_calendar_request:
    "El helper rechazó los parámetros de la lectura de calendario. Corrígelos antes de volver a pedirla.",
  invalid_screen_inventory:
    "El inventario de ventanas no cumple el contrato del helper. Vuelve a pedir las ventanas.",
  invalid_screenshot_dimensions:
    "La captura tiene dimensiones no válidas; no se entregó. Vuelve a pedirla si sigue haciendo falta.",
  invalid_window_binding:
    "La identidad de la ventana pedida no es válida. Vuelve a pedir las ventanas y elige una de ellas.",
  invalid_window_dimensions:
    "La ventana tiene dimensiones no válidas para capturar. Vuelve a pedir las ventanas y elige otra.",
  privacy_stopped:
    "Mars detuvo la privacidad en ese Mac: Orbit no ve ni actúa en él hasta que ella lo rearme allí. No es un error del Mac ni de la conexión; pregunta de nuevo cuando Mars lo rearme.",
  screen_inventory_busy:
    "El helper sigue enumerando las ventanas para otra petición. Espera a que termine antes de pedirlas otra vez.",
  screen_inventory_too_large:
    "El inventario de ventanas supera el tamaño que entrega el helper. No se entregó una lista parcial.",
  screen_permission_required:
    "macOS no concede a Orbit Grabación de pantalla en ese Mac. Se concede en Ajustes del Sistema › Privacidad y seguridad › Grabación de pantalla.",
  screen_selection_expired:
    "La selección de ventana ha caducado. Vuelve a pedir las ventanas antes de capturar.",
  screen_selection_unavailable:
    "El Mac no puede seleccionar esa ventana ahora. Vuelve a pedir las ventanas.",
  screenshot_busy:
    "El helper está terminando otra captura. Espera a que termine antes de pedir otra.",
  screenshot_expired:
    "La captura no terminó dentro del plazo del helper. Vuelve a pedirla si sigue haciendo falta.",
  screenshot_sdk_failed:
    "ScreenCaptureKit no pudo obtener la imagen de esa ventana. Vuelve a pedir la captura si sigue haciendo falta.",
  screenshot_too_large:
    "El helper no pudo codificar la imagen PNG dentro del tamaño de captura admitido; no se entregó la imagen.",
  selected_text_changed:
    "La selección de texto cambió mientras se leía. Vuelve a pedir el texto seleccionado.",
  selected_text_classification_incomplete:
    "El Mac no pudo comprobar si el campo seleccionado es seguro. No se leyó su texto.",
  selected_text_empty:
    "Accesibilidad confirma un rango de selección vacío: no hay texto seleccionado en ese campo.",
  selected_text_expired:
    "La lectura del texto seleccionado caducó. Vuelve a pedirla si sigue haciendo falta.",
  selected_text_identity_changed:
    "La ventana o su selección cambió durante la lectura. Vuelve a pedir el texto seleccionado.",
  selected_text_invalid_range:
    "El rango del texto seleccionado no es válido. Vuelve a pedir la selección.",
  selected_text_invalid_text:
    "El texto que entregó Accesibilidad no es válido; no se usó. Vuelve a pedirlo.",
  selected_text_limit:
    "La selección supera el límite de lectura del helper. Pide una selección más corta.",
  selected_text_not_exposed:
    "La aplicación no expone los atributos de Accesibilidad necesarios para leer la selección; no se sabe si hay texto seleccionado.",
  selected_text_secure_field:
    "La selección está en un campo protegido y Orbit no lo lee. Pide otro campo.",
  selected_text_unavailable:
    "Accesibilidad no pudo entregar la selección de esa ventana; no se sabe si hay texto seleccionado.",
  selected_text_window_ambiguous:
    "Accesibilidad no pudo distinguir la ventana pedida de otras. Vuelve a pedir las ventanas y selecciona una inequívoca.",
  window_identity_changed:
    "La ventana cambió entre enumerarla y leerla. Vuelve a pedir la ventana o la captura.",
};

export function fields(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};
}

function expectedText(value: unknown): string {
  const expected = fields(value);
  const kind = expected.kind;
  if (kind === "wire_type") {
    const names: Record<string, string> = {
      string: "texto",
      object: "un objeto",
      boolean: "verdadero o falso",
    };
    return names[String(expected.name)] ?? `el contrato ${String(expected.name)}`;
  }
  if (kind === "calendar_event")
    return `un evento válido en la fila ${String(expected.position)} de ${String(expected.rows)}`;
  if (kind === "signed_range" || kind === "unsigned_range")
    return `un valor entre ${String(expected.min)} y ${String(expected.max)}; se recibió ${String(expected.received)}`;
  if (kind === "text_length")
    return `texto no vacío de hasta ${String(expected.max_bytes)} bytes; se recibieron ${String(expected.bytes)}`;
  if (kind === "read_duration")
    return `una lectura de menos de ${String(expected.max_exclusive_ns)} ns; duró ${String(expected.read_ns)} ns`;
  if (kind === "read_times")
    return `tiempos ordenados; empezó en ${String(expected.started_ns)} ns y terminó en ${String(expected.completed_ns)} ns`;
  if (kind === "current_generation")
    return `la generación ${String(expected.required ?? "sin observar")}; se recibió ${String(expected.received)} y la instancia ${expected.instance_matches === true ? "coincide" : "no coincide"}`;
  if (kind === "clock_domain")
    return `el reloj del helper; se recibió ${String(expected.received)}`;
  return "el contrato de esa lectura";
}

function receivedType(value: unknown): string {
  const names: Record<string, string> = {
    string: "texto",
    number: "número",
    boolean: "booleano",
    object: "objeto",
    array: "lista",
    null: "nulo",
    absent: "sin dato",
  };
  return names[String(value)] ?? "dato no válido";
}

export function helperRefused(refusal: Refusal): string {
  return (
    helperMessages[refusal.helper_code as HelperCode] ??
    "El helper rechazó la lectura por una causa que esta versión no reconoce."
  );
}

export function helperReplyInvalid(refusal: Refusal): string {
  return `La respuesta del helper no cumple el contrato: ${String(refusal.field)} debe ser ${expectedText(refusal.expected)}; tipo recibido: ${receivedType(refusal.received_type)}.`;
}
