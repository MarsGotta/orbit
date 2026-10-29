"""Invented spoken/input content, separate from Orbit's catalogue labels."""

from pathlib import Path

from harness_copy import language


def script_path() -> Path:
    name = "meeting-script-en.txt" if language() == "en" else "meeting-script.txt"
    return Path(__file__).resolve().with_name(name)


def question() -> str:
    return "what are we discussing" if language() == "en" else "de qué estamos hablando"


def voice_note_phrase() -> str:
    """Not a line of the meeting script: check 1 counts every one of its words
    in the meeting's transcript. Deimos is asked for one word so the answer is
    short."""
    return (
        "Deimos, respond with only the word received."
        if language() == "en"
        else "Deimos, contesta sólo con la palabra recibido."
    )


def chat_message(ident: str) -> str:
    return (
        f"Orbit test {ident}: respond with only the word received."
        if language() == "en"
        else f"Prueba de Orbit {ident}: contesta sólo con la palabra recibido."
    )


def draft_message(ident: str) -> str:
    return (
        f"Test draft {ident}: written immediately before quitting"
        if language() == "en"
        else f"Borrador de prueba {ident}: lo escrito justo antes de salir"
    )
