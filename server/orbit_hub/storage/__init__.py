"""The hub's durable data: one encrypted SQLite database, a repository per domain.

`Store` (`database.py`) is the database itself. Each domain keeps its rows
through its own repository over that one connection and one schema
(`schema.py`), with no network or provider dependencies:

- `store.devices`: paired devices and pairing codes (`devices.py`);
- `store.chat`: conversations and their messages (`chat.py`);
- `store.operations`: the durable receipts of what a device asked for
  (`operations.py`);
- `store.recordings`: audio chunks and the manifest that closes a recording
  (`recordings.py`); `store.groups`, the groups they are transcribed in
  (`group_storage.py`);
- `store.space`: what the database uses, its limit and admission (`space.py`);
- `store.reminders`: the owner's «Recordatorio personal» of each meeting,
  shared by every Mac (`reminders.py`);
- `store.attachments`, and the meetings, notes and agents repositories, which
  take the store.
"""

from .chat import SESSION_LIST_LIMIT, SESSION_MESSAGES_PAGE_BYTES, SESSION_MESSAGES_PAGE_LIMIT
from .database import Store, initialize
from .operations import OPERATION_RECEIPT_RESERVE_BYTES
from .space import STORAGE_FREE_FLOOR_BYTES, STORAGE_WARNING_PERCENT, is_storage_full

__all__ = (
    "OPERATION_RECEIPT_RESERVE_BYTES",
    "SESSION_LIST_LIMIT",
    "SESSION_MESSAGES_PAGE_BYTES",
    "SESSION_MESSAGES_PAGE_LIMIT",
    "STORAGE_FREE_FLOOR_BYTES",
    "STORAGE_WARNING_PERCENT",
    "Store",
    "initialize",
    "is_storage_full",
)
