"""The note as the user sees it: what the model wrote, with their own edits applied.

Pure: no store, clock or network. The repository keeps the model's note and
the user's edits apart so that a regeneration never overwrites their work
silently; whoever reads the note on their behalf (Deimos, through the meetings
bridge) must see the result of both, never the model's version alone.
"""


def as_seen(note: dict, edits: dict | None) -> tuple[dict, dict]:
    """The note with the user's title, sections and done marks, and what they edited.

    An edited title or section list replaces the model's whole; a done mark
    applies to the task with that id, and a task without a mark is not done.
    """
    edits = edits or {}
    title = edits.get("title")
    sections = edits.get("sections")
    done = edits.get("done") or {}
    seen = {
        **note,
        "title": title if title is not None else note["title"],
        "sections": sections if sections is not None else note["sections"],
        "action_items": [
            {**item, "done": done.get(item["id"]) is True} for item in note["action_items"]
        ],
    }
    return seen, {"title": title is not None, "sections": sections is not None}
