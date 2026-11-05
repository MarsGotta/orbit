"""Check 26's vault calls, through Orbit Pruebas's own daemon. Mac-side Python 3.9.

The bodies are the app's own (`orbit_vault`, Command::Vault): the connection
Settings › Notes saves, and the list of a meeting's proposals that screen 53
and the meeting's pill read.
"""

from __future__ import annotations

import json

from mac_controls import MacFailure

# What a proposal passes through by itself: a wait never ends on one of these.
MOVING = ("queued", "preparing", "applying", "undoing")


class VaultCalls:
    def vault(self, call: dict) -> dict:
        """One vault call; a refusal is said with its cause."""
        reply = self.orbit.call({"command": "vault", "call": call})
        outcome = reply.get("outcome") or {}
        if outcome.get("outcome") in (None, "refused", "failed"):
            raise MacFailure(
                f"the daemon refused vault «{call.get('call')}»: "
                f"{json.dumps(outcome or reply, ensure_ascii=False)[:300]}"
            )
        return outcome

    def distillation(self, meeting: str, after: str, max_s: float) -> dict:
        """The meeting's newest proposal once it has settled in a status other
        than `after` (`none` before there is one): proposed or failed after the
        hub prepares it, applied or stale after Apply, undone after Undo."""
        from mac import READ_INTERVAL_S

        start = self.system.now()
        seen = "none"
        while True:
            page = self.vault({"call": "list_distillations", "meeting_id": meeting, "page": 1})
            entries = (page.get("page") or {}).get("entries") or []
            if entries:
                seen = entries[0].get("status")
                if seen not in MOVING and seen != after:
                    return {
                        "proposal": entries[0],
                        "seconds": round(self.system.now() - start, 3),
                    }
            if self.system.now() - start >= max_s:
                raise MacFailure(
                    f"the meeting's proposal did not settle past «{after}» within "
                    f"{max_s:.0f} s; last seen: {seen}"
                )
            self.system.pause(READ_INTERVAL_S)
