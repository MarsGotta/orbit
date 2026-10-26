"""Every paired device's runtime, as the hub holds them."""

import asyncio
from collections.abc import Callable
from concurrent.futures import Executor

from ..assistant.agents import Agents
from ..protocol import emit
from ..storage import Store
from .device import DeviceRuntime
from .inputs import TurnInputTracker


class Runtimes(dict[str, DeviceRuntime]):
    """One runtime per device, by device id, made when its first socket asks.

    The device bridge and the routes read it as the dict it is; a viewer can
    disconnect while its turn goes on, so a runtime lives until its device is
    revoked (`revoke_inactive`) or the hub stops."""

    def __init__(
        self,
        store: Store,
        agents: Agents,
        now: Callable[[], float],
        store_executor: Executor,
        inputs: TurnInputTracker,
    ):
        super().__init__()
        self.store, self.agents, self.now = store, agents, now
        self.store_executor, self.inputs = store_executor, inputs

    def of(self, device: str) -> DeviceRuntime:
        """The device's runtime, whichever authenticated socket asks first."""
        return self.setdefault(
            device,
            DeviceRuntime(
                device,
                self.store,
                self.agents,
                self.now,
                store_executor=self.store_executor,
                input_tracker=self.inputs,
            ),
        )

    def offer_all(self, frame: dict) -> None:
        """Every connected Mac gets `frame`."""
        for runtime in list(self.values()):
            runtime.offer(frame)

    def agents_changed(self) -> None:
        """Every connected Mac learns the new list, as `ready` would give it."""
        self.offer_all(emit("agents.changed", self.agents.listing()))

    def recording_changed(self, device: str, recording: str) -> None:
        """Every connected Mac reads the person's changed meeting again."""
        self.offer_all(emit("recording.changed", {"recording_id": recording}))

    async def close(self) -> None:
        await asyncio.gather(*(runtime.close() for runtime in self.values()))
