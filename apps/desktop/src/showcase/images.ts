/**
 * The bytes of the invented attachments, drawn here so nothing real is
 * shown: a chart of the pilot's occupancy, a whiteboard photo and a
 * one-page PDF.
 */

function canvas(width: number, height: number) {
  const element = document.createElement("canvas");
  element.width = width;
  element.height = height;
  const context = element.getContext("2d");
  if (!context) throw new Error("canvas 2d unavailable");
  return { element, context };
}

function png(element: HTMLCanvasElement): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) =>
    element.toBlob(
      (blob) => (blob ? resolve(blob.arrayBuffer()) : reject(new Error("png"))),
      "image/png",
    ),
  );
}

function chart(): Promise<ArrayBuffer> {
  const { element, context } = canvas(1200, 760);
  context.fillStyle = "#ffffff";
  context.fillRect(0, 0, 1200, 760);
  context.fillStyle = "#1f2430";
  context.font = "bold 40px sans-serif";
  context.fillText("Ocupación del piloto · semana 44", 60, 90);
  const values = [72, 64, 81, 58, 90, 95, 77];
  const days = ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"];
  values.forEach((value, index) => {
    const x = 90 + index * 150;
    const height = value * 5.2;
    context.fillStyle = index >= 5 ? "#e8813a" : "#6a5cff";
    context.fillRect(x, 660 - height, 90, height);
    context.fillStyle = "#1f2430";
    context.font = "28px sans-serif";
    context.fillText(`${value}%`, x + 10, 645 - height);
    context.fillText(days[index] ?? "", x + 20, 710);
  });
  return png(element);
}

function whiteboard(): Promise<ArrayBuffer> {
  const { element, context } = canvas(1000, 700);
  const gradient = context.createLinearGradient(0, 0, 1000, 700);
  gradient.addColorStop(0, "#f4f1ea");
  gradient.addColorStop(1, "#dcd6c8");
  context.fillStyle = gradient;
  context.fillRect(0, 0, 1000, 700);
  context.strokeStyle = "#2b5fd9";
  context.lineWidth = 6;
  ["Elegir fechas", "Habitación", "Pago"].forEach((label, index) => {
    const x = 80 + index * 300;
    context.strokeRect(x, 260, 230, 140);
    context.fillStyle = "#2b5fd9";
    context.font = "34px sans-serif";
    context.fillText(label, x + 20, 340);
    if (index < 2) {
      context.beginPath();
      context.moveTo(x + 240, 330);
      context.lineTo(x + 290, 330);
      context.stroke();
    }
  });
  context.fillStyle = "#c2412d";
  context.font = "italic 38px sans-serif";
  context.fillText("¿pago en una sola pantalla?", 300, 520);
  return png(element);
}

/** A valid one-page PDF, its offsets counted as it is written. */
function pdf(): ArrayBuffer {
  const text =
    "BT /F1 22 Tf 72 760 Td (Propuesta de piloto - Hoteles Ribera) Tj 0 -36 Td /F1 13 Tf (Cinco hoteles, del 26 de octubre al 30 de noviembre.) Tj ET";
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
    `<< /Length ${text.length} >>\nstream\n${text}\nendstream`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  let body = "%PDF-1.4\n";
  const offsets = objects.map((object, index) => {
    const offset = body.length;
    body += `${index + 1} 0 obj\n${object}\nendobj\n`;
    return offset;
  });
  const xref = body.length;
  body += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  body += offsets.map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("");
  body += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return new TextEncoder().encode(body).buffer;
}

const drawn = new Map<string, Promise<ArrayBuffer>>();

/** The bytes of an invented attachment, by its id. */
export function attachmentBytes(id: string): Promise<ArrayBuffer> {
  const known = drawn.get(id);
  if (known) return known;
  const bytes = id.startsWith("pdf")
    ? Promise.resolve(pdf())
    : id.startsWith("photo")
      ? whiteboard()
      : chart();
  drawn.set(id, bytes);
  return bytes;
}
