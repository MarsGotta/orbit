"""Notes as a plugin of the hub: its manifest, its routes and its meeting table."""

from pathlib import Path

from ..distillation import mount as mount_distillations
from ..plugins import Plugin, read_manifest
from .routes import mount as mount_vault

MANIFEST = Path(__file__).with_name("orbit-plugin.json")
# The proposals name the meeting they were prepared from: they go with it.
RECORDING_TABLES = ("vault_distillations",)


def notes() -> Plugin:
    return Plugin(read_manifest(MANIFEST), mount, RECORDING_TABLES)


def mount(app, hub, identity, bounded_body, transport) -> None:
    """The vault's routes, the signed read included, then the distillation's."""
    store, distillations = hub.store, hub.distillations
    mount_vault(
        app, store, identity, bounded_body, hub.meetings_bridge, transport, distillations.model
    )
    app.state.distillations = distillations
    mount_distillations(app, distillations, identity, bounded_body)
