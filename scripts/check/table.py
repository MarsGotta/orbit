#!/usr/bin/env python3
"""Prints the final table of scripts/check.sh from its `resultados.tsv`.

Columns of the TSV, one step per line: id, result (ok | fallo | omitido |
interrumpido), seconds, label, cause.

A row carries its cause cut to a readable width; every cause is then listed
whole under the table, so a long list of failed tests never hides the table
and never loses a name.

Usage: table.py RESULTS.tsv TOTAL_SECONDS
"""

import sys
from pathlib import Path

SHOWN = {"ok": "ok", "fallo": "FALLO", "omitido": "omitido", "interrumpido": "INTERRUMPIDO"}
# Characters of a cause shown inside its row; the whole cause follows the table.
ROW_CAUSE = 60


def duration(seconds: float) -> str:
    if seconds < 60:
        return f"{seconds:.1f} s"
    minutes, rest = divmod(round(seconds), 60)
    if minutes < 60:
        return f"{minutes} min {rest:02d} s"
    hours, minutes = divmod(minutes, 60)
    return f"{hours} h {minutes:02d} min"


def main(arguments: list[str]) -> int:
    results, total = Path(arguments[0]), float(arguments[1])
    rows = []
    for line in results.read_text(encoding="utf-8").splitlines():
        _, result, seconds, label, cause = (line.split("\t") + [""] * 5)[:5]
        status = SHOWN.get(result, result)
        shown = status
        if cause:
            short = cause if len(cause) <= ROW_CAUSE else cause[: ROW_CAUSE - 1] + "…"
            shown = f"{status} ({short})"
        rows.append((label, shown, duration(float(seconds or 0)), result, status, cause))
    passed = sum(1 for row in rows if row[3] == "ok")
    verdict = "ok" if rows and passed == len(rows) else "FALLO"
    summary = f"{len(rows)} pasos: {passed} ok, {len(rows) - passed} no"
    header = ("Paso", "Resultado", "Duración")
    label_width = max([len(header[0]), len(summary)] + [len(row[0]) for row in rows])
    result_width = max([len(header[1]), len(verdict)] + [len(row[1]) for row in rows])
    rule = "─" * (label_width + result_width + 16)
    print()
    print(rule)
    print(f"{header[0]:<{label_width}}  {header[1]:<{result_width}}  {header[2]:>10}")
    print(rule)
    for label, shown, spent, *_ in rows:
        print(f"{label:<{label_width}}  {shown:<{result_width}}  {spent:>10}")
    print(rule)
    print(f"{summary:<{label_width}}  {verdict:<{result_width}}  {duration(total):>10}")
    causes = [(label, status, cause) for label, _, _, result, status, cause in rows if cause]
    if causes:
        print("\nCausas:")
        for label, status, cause in causes:
            print(f"  {label} · {status}: {cause}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
