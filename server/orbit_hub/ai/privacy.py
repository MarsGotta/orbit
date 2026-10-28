"""Privacy copy codes and English fallback text for older Macs."""

from dataclasses import dataclass
from urllib.parse import urlsplit

from ..urls import LOOPBACK_HOSTS
from .types import MAI


@dataclass(frozen=True)
class Privacy:
    text: str
    copies: list[dict]


def sentence(code, *, material=None, destination=None):
    values = {
        k: v for k, v in {"material": material, "destination": destination}.items() if v is not None
    }
    text = {
        "openrouter": f"The {material} is sent to {destination}.",
        "network": f"The {material} is sent to {destination}, outside your hub, over your network or the Internet.",
        "hub": f"The {material} stays on your hub.",
        "operator": "The operator configuration is used; the service may send data outside the hub.",
        "vault_legacy": "Notes model saved by the operator; the hub job does not run yet.",
        "unconfigured": "No operator model is configured for this function.",
        "hermes": f"The {material} is sent to Hermes at {destination}; its configuration determines whether data goes to another provider.",
    }[code]
    return Privacy(text, [{"code": code, **values}])


def describe_privacy(function, provider, model):
    material = "audio" if function in ("meeting_transcription", "listening") else "text"
    if provider["kind"] == "openrouter":
        destination = "microsoft_openrouter" if model == MAI else "openrouter_provider"
        result = sentence("openrouter", material=material, destination=destination)
        target = (
            "Microsoft through OpenRouter" if model == MAI else "OpenRouter and the model provider"
        )
        return Privacy(f"The {material} is sent to {target}.", result.copies)
    url = provider.get("base_url")
    if url and urlsplit(url).hostname not in LOOPBACK_HOSTS:
        return sentence("network", material=material, destination=provider["name"])
    if url or provider["kind"] == "hub":
        return sentence("hub", material=material)
    return sentence("operator")


def combine(descriptions):
    unique = {entry.text: entry for entry in descriptions}.values()
    return Privacy(
        " ".join(entry.text for entry in unique),
        [copy for entry in unique for copy in entry.copies],
    )
