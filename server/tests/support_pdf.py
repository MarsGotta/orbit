"""Small PDFs written by hand for the attachment tests: invented text, no real document."""


def pdf_bytes(pages: list[str]) -> bytes:
    """A PDF with one page per item: the item's text in Helvetica, or, for an
    empty item, only a filled rectangle (a page a scan or a figure would be)."""
    objects: list[bytes] = []
    page_ids = [3 + 2 * index for index in range(len(pages))]
    font_id = 3 + 2 * len(pages)
    objects.append(b"<< /Type /Catalog /Pages 2 0 R >>")
    kids = " ".join(f"{page} 0 R" for page in page_ids)
    objects.append(f"<< /Type /Pages /Kids [{kids}] /Count {len(pages)} >>".encode())
    for index, text in enumerate(pages):
        content_id = page_ids[index] + 1
        objects.append(
            (
                f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] "
                f"/Resources << /Font << /F1 {font_id} 0 R >> >> /Contents {content_id} 0 R >>"
            ).encode()
        )
        if text:
            lines = []
            for number, line in enumerate(text.split("\n")):
                escaped = line.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
                lines.append(f"BT /F1 12 Tf 20 {370 - 16 * number} Td ({escaped}) Tj ET")
            stream = "\n".join(lines).encode("latin-1")
        else:
            stream = b"0.2 0.4 0.6 rg 40 60 220 280 re f"
        objects.append(
            b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"\nendstream"
        )
    objects.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    out = bytearray(b"%PDF-1.4\n")
    offsets = []
    for number, body in enumerate(objects, 1):
        offsets.append(len(out))
        out += f"{number} 0 obj\n".encode() + body + b"\nendobj\n"
    xref = len(out)
    out += f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode()
    for offset in offsets:
        out += f"{offset:010d} 00000 n \n".encode()
    out += (
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ).encode()
    return bytes(out)
