"""Translate the good replay by catalogue key, without duplicating its screens.

Meeting/model prose is invented fixture content, kept apart from UI labels.
A single simultaneous replacement prevents a translation from being translated
again, and preserves the indices/roles used by the same driver and checks.
"""

from __future__ import annotations

import json
import re
from functools import lru_cache
from pathlib import Path

from harness_copy import NATIVE, UI, language_scope
from mac_controls import script_parts
from meeting_language import question, script_path, voice_note_phrase


@lru_cache(maxsize=1)
def replacements():
    mapping = {}
    for key, spanish in UI["es"].items():
        english = UI["en"][key]
        if len(spanish) >= 3 and "{" not in spanish and spanish != english:
            mapping.setdefault(spanish, english)
    for key, spanish in NATIVE["es"].items():
        english = NATIVE["en"][key]
        if isinstance(spanish, str) and spanish != english and "{" not in spanish:
            mapping.setdefault(spanish, english)
    parts = []
    questions, phrases = [], []
    for selected in ("es", "en"):
        with language_scope(selected):
            parts.append(script_parts(script_path().read_text(encoding="utf-8")))
            questions.append(question())
            phrases.append(voice_note_phrase())
    for spanish, english in zip(parts[0], parts[1], strict=True):
        for (_, source), (_, target) in zip(spanish, english, strict=True):
            mapping[source] = target
    mapping[questions[0]] = questions[1]
    mapping[phrases[0]] = phrases[1]
    # This is the language of this meeting, not the option to choose Spanish.
    mapping[UI["es"]["notes.languages.es"]] = UI["en"]["notes.languages.en"]
    content = Path(__file__).with_name("dry-language-content.json")
    mapping.update(json.loads(content.read_text(encoding="utf-8")))
    # JSON's domain language field follows the selected synthetic meeting.
    mapping['"language": "es"'] = '"language": "en"'
    pattern = re.compile(
        "|".join(
            (r"(?<!\w)" if value[0].isalnum() else "")
            + re.escape(value)
            + (r"(?!\w)" if value[-1].isalnum() else "")
            for value in sorted(mapping, key=len, reverse=True)
        )
    )
    return pattern, mapping


def translate(value: str, language: str) -> str:
    if language == "es":
        return value
    if language != "en":
        raise ValueError(f"unsupported replay language {language!r}")
    for key in (
        "shellInline.todayAt",
        "shellInline.yesterdayAt",
        *(
            f"{kind}.{month}"
            for kind in ("sessionDateYear", "sessionDate")
            for month in range(1, 13)
        ),
    ):
        source = re.escape(UI["es"][key])
        for slot, pattern in (
            ("day", r"(?P<day>\d{1,2})"),
            ("year", r"(?P<year>\d{4})"),
            ("time", r"(?P<hour>\d{2}):(?P<minute>\d{2})"),
        ):
            source = source.replace(re.escape("{" + slot + "}"), pattern)

        def date(match, key=key):
            values = match.groupdict()
            values["time"] = UI["en"][f"sessionClock.{int(values.pop('hour'))}"].format(
                minute=values.pop("minute")
            )
            return UI["en"][key].format(**values)

        value = re.sub(source, date, value)
    pattern, mapping = replacements()
    return pattern.sub(lambda match: mapping[match[0]], value)
