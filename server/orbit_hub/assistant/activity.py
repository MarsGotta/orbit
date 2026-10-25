"""What the agent does in a turn, as the Mac reads it: `chat.item` (hub-v1).

The backends translate their own events into `ActivityItem`s (`backend`);
this module writes the public item: a bounded one-line label, in Spanish for
Orbit's own tools (orbit-mcp) and the tool's own name for any other.
"""

from .backend import ActivityItem, PlanStep

# A label is one line the chat shows while a step runs, or the reasoning
# summary folded under it. 500 characters is Hermes's own reasoning preview
# (`reasoning.available`); past that it is no longer a line read in passing.
# It protects the frame and the Mac's assistant topic from upstream text of
# any length (a goal, a raw tool name, a thought). It does not limit what the
# agent does or says: the answer streams as `chat.delta`.
MAX_ITEM_LABEL_CHARS = 500
# What a tool step used, for the summary («Usó la pantalla y el calendario»):
# a noun phrase, or a tool's own name, which upstream bounds nowhere.
MAX_ITEM_SUBJECT_CHARS = 80
# Why a step failed, in the agent's words: one line, as `the vendor refusal bound`.
MAX_ITEM_CAUSE_CHARS = 300
# The steps of a plan the Mac lists. Each update carries the whole plan, so
# this bounds every frame of it; the label still counts every step.
MAX_PLAN_STEPS = 20

# orbit-mcp's seventeen tools, by the name the agent calls (an MCP client may
# prefix it with the server's name: `mcp__orbit__orbit_look`, `mcp_orbit_orbit_look`).
ORBIT_TOOLS = (
    "orbit_devices",
    "orbit_windows",
    "orbit_screenshot",
    "orbit_look",
    "orbit_selected_text",
    "orbit_control_tools",
    "orbit_perform_action",
    "orbit_action_result",
    "orbit_calendar",
    "orbit_meetings",
    "orbit_transcript",
    "orbit_note",
    "orbit_files_list",
    "orbit_files_search",
    "orbit_files_read",
    "orbit_attachment",
    "orbit_skill",
)

_FIXED = {
    "orbit_devices": ("Mirando los Mac de Orbit", "los Mac"),
    "orbit_windows": ("Mirando las ventanas", "las ventanas"),
    "orbit_look": ("Mirando la pantalla", "la pantalla"),
    "orbit_selected_text": ("Leyendo el texto seleccionado", "el texto seleccionado"),
    "orbit_control_tools": ("Viendo qué puede hacer en el Mac", "el Mac"),
    "orbit_action_result": ("Comprobando cómo acabó la acción", "el Mac"),
    "orbit_calendar": ("Leyendo el calendario", "el calendario"),
    "orbit_meetings": ("Leyendo las reuniones", "las reuniones"),
    "orbit_transcript": ("Leyendo la transcripción de una reunión", "las reuniones"),
    "orbit_note": ("Leyendo la nota de una reunión", "las reuniones"),
    # Never the path or the search: they can name the user's private documents.
    "orbit_files_list": ("Mirando las carpetas del Mac", "las carpetas"),
    "orbit_files_search": ("Buscando en las carpetas del Mac", "las carpetas"),
    "orbit_files_read": ("Leyendo un archivo del Mac", "las carpetas"),
    # Never the file's name: it is the user's words.
    "orbit_attachment": ("Leyendo un adjunto del mensaje", "los adjuntos"),
    # The agent's own configuration, nothing of the user's.
    "orbit_skill": ("Cargando una de sus skills", "sus skills"),
}


def one_line(text: object, limit: int) -> str:
    """`text` on one line, its runs of space and control characters made one
    space, cut to `limit` characters with an ellipsis."""
    if not isinstance(text, str):
        return ""
    words = "".join(" " if ord(char) < 32 or char == "\x7f" else char for char in text).split()
    line = " ".join(words)
    if len(line) <= limit:
        return line
    return line[: limit - 1] + "…"


def orbit_tool(name: str) -> str | None:
    """The orbit-mcp tool `name` calls, whatever prefix its client added."""
    for tool in ORBIT_TOOLS:
        if name == tool or name.endswith(("__" + tool, "/" + tool, "_" + tool, "." + tool)):
            return tool
    return None


def _app(arguments: object) -> str:
    if not isinstance(arguments, dict):
        return ""
    for key in ("app", "app_name", "application", "name", "bundle_id"):
        if value := one_line(arguments.get(key), MAX_ITEM_SUBJECT_CHARS):
            return value
    return ""


def _action_label(arguments: dict) -> str:
    action = arguments.get("action")
    action = action if isinstance(action, dict) else {}
    tool = one_line(action.get("tool"), MAX_ITEM_SUBJECT_CHARS)
    app = _app(action.get("arguments"))
    if "click" in tool:
        return f"Pulsando en {app}" if app else "Pulsando en la pantalla"
    if tool.startswith("type"):
        # Never the text typed: it can be anything the user has on screen.
        return "Escribiendo"
    if tool in {"press_key", "hotkey"}:
        return "Pulsando teclas"
    if tool == "scroll":
        return f"Desplazándose por {app}" if app else "Desplazándose"
    if tool in {"launch_app", "bring_to_front"}:
        return f"Abriendo {app}" if app else "Abriendo una app"
    return f"Usando {tool} en el Mac" if tool else "Actuando en el Mac"


def tool_label(name: str, arguments: dict | None) -> tuple[str, str]:
    """The line a tool step shows while it runs, and what it used."""
    arguments = arguments if isinstance(arguments, dict) else {}
    tool = orbit_tool(name)
    if tool is None:
        own = one_line(name, MAX_ITEM_SUBJECT_CHARS) or "herramienta"
        return own, own
    if tool in _FIXED:
        return _FIXED[tool]
    if tool == "orbit_screenshot":
        app = one_line(arguments.get("application"), MAX_ITEM_SUBJECT_CHARS)
        return (f"Mirando {app}" if app else "Mirando la pantalla"), "la pantalla"
    return _action_label(arguments), "el Mac"


def _plan_label(steps: tuple[PlanStep, ...]) -> str:
    done = sum(step.status in {"completed", "cancelled"} for step in steps)
    return f"Plan · {done} de {len(steps)} pasos"


def public_item(event: ActivityItem) -> dict:
    """The `chat.item` data of one step, without its session id."""
    data: dict = {"item_id": event.item_id, "kind": event.kind, "phase": event.phase}
    if event.kind == "tool":
        label, subject = tool_label(event.tool or "", event.arguments)
        data["label"] = one_line(label, MAX_ITEM_LABEL_CHARS)
        data["subject"] = subject
    elif event.kind == "plan":
        data["label"] = _plan_label(event.steps)
        data["steps"] = [
            {"text": one_line(step.text, MAX_ITEM_LABEL_CHARS), "status": step.status}
            for step in event.steps[:MAX_PLAN_STEPS]
        ]
    else:
        data["label"] = one_line(event.text, MAX_ITEM_LABEL_CHARS)
    if event.phase == "failed":
        data["cause"] = one_line(event.cause, MAX_ITEM_CAUSE_CHARS) or "sin causa declarada"
    return data
