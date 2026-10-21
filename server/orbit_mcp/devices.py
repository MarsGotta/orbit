"""Which Mac a tool call is about: the hub's listing, the file's names and the default."""

from __future__ import annotations

import time
import unicodedata
from dataclasses import dataclass

from orbit_hub.protocol import DeviceSummary

from .config import Config
from .hub import HubClient, HubClientError


@dataclass(frozen=True)
class DeviceSelection:
    requested_name: str
    device_id: str
    device: DeviceSummary | None
    same_name: tuple[DeviceSummary, ...] = ()
    notice: str | None = None


class UnknownDevice(HubClientError):
    def __init__(self, devices: list[DeviceSummary], default_device_id: str | None):
        super().__init__("unknown_device")
        self.devices = tuple(devices)
        self.default_device_id = default_device_id


def seen_ago(timestamp: float) -> str:
    seconds = max(0, time.time() - timestamp)
    if seconds < 60:
        return "hace menos de un minuto"
    return f"hace {int(seconds // 60)} min"


def folded_name(value: str) -> str:
    decomposed = unicodedata.normalize("NFKD", value.casefold())
    return "".join(character for character in decomposed if not unicodedata.combining(character))


def validated_devices(rows: list[dict]) -> list[DeviceSummary]:
    try:
        return [DeviceSummary.model_validate(row) for row in rows]
    except (ValueError, TypeError):
        raise HubClientError("invalid_response") from None


def hub_selection(devices: list[DeviceSummary], selected: str) -> DeviceSelection:
    normalized = folded_name(selected)
    exact = [device for device in devices if folded_name(device.name) == normalized]
    if exact:
        matches = exact
    else:
        matching_names = {
            folded_name(device.name)
            for device in devices
            if folded_name(device.name).startswith(normalized)
        }
        if len(matching_names) != 1:
            raise HubClientError("unknown_device")
        matches = [device for device in devices if folded_name(device.name) in matching_names]
    connected = [device for device in matches if device.connected]
    candidates = connected or matches
    chosen = max(candidates, key=lambda device: (device.paired_at, device.last_seen, device.id))
    notice = None
    if len(matches) > 1:
        choice = (
            "la identidad conectada más reciente"
            if connected
            else "la identidad disponible más reciente"
        )
        notice = f"Hay varias identidades con el nombre «{chosen.name}»; se usó {choice}."
    return DeviceSelection(selected, chosen.id, chosen, tuple(matches), notice)


async def device_selection(
    config: Config, hub_client: HubClient, name: str | None
) -> DeviceSelection:
    selected = name or config.default_device
    try:
        rows = await hub_client.devices(fresh=True) if name is None else await hub_client.devices()
        devices = validated_devices(rows)
    except HubClientError:
        if selected not in config.devices:
            raise
        return DeviceSelection(
            selected,
            config.devices[selected],
            None,
            notice=(
                f"Se usó «{selected}», del fichero; el hub no pudo informar de los Mac."
                if name is None
                else None
            ),
        )
    if name is None:
        connected = [device for device in devices if device.connected]
        active = [device for device in connected if device.assistant_turn_active is True]
        if len(active) == 1:
            device = active[0]
            return DeviceSelection(
                device.name,
                device.id,
                device,
                notice=f"Se usó «{device.name}»: tiene el turno de Deimos en curso.",
            )
        if len(connected) == 1:
            device = connected[0]
            return DeviceSelection(
                device.name,
                device.id,
                device,
                notice=f"Se usó «{device.name}»: es el único Mac conectado.",
            )
    if selected in config.devices:
        device_id = config.devices[selected]
        device = next((candidate for candidate in devices if candidate.id == device_id), None)
        reference_name = device.name if device is not None else selected
        same_name = tuple(
            candidate
            for candidate in devices
            if folded_name(candidate.name) == folded_name(reference_name)
        )
        selection = DeviceSelection(selected, device_id, device, same_name)
    else:
        try:
            selection = hub_selection(devices, selected)
        except HubClientError as exc:
            if exc.code != "unknown_device":
                raise
            default_id = config.devices.get(config.default_device)
            if default_id is None:
                try:
                    default_id = hub_selection(devices, config.default_device).device_id
                except HubClientError:
                    pass
            raise UnknownDevice(devices, default_id) from None
    if name is None:
        chosen = selection.device.name if selection.device is not None else selected
        notice = f"Se usó «{chosen}»: es el Mac predeterminado del fichero."
        if selection.notice:
            notice += " " + selection.notice
        return DeviceSelection(
            selection.requested_name,
            selection.device_id,
            selection.device,
            selection.same_name,
            notice,
        )
    return selection
