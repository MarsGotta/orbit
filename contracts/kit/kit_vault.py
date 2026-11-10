"""A hub of the kit's own for the vault tests.

One hub for every vault test: a new folder with a new storage key and no
database, a device the kit pairs (`pair-code`, then `POST /v1/pair`) and
`serve` until the tests end. Each test points it at a vault folder it writes.
The hub reads that folder from its own disk, so it runs on the kit's machine.
"""

import secrets
from collections.abc import Iterator

import pytest
from kit_hub import KIT_DEVICE_NAME, pair_device, prepare, run, serving

# The size of the storage key a hub reads from its key file.
STORAGE_KEY_BYTES = 32


@pytest.fixture(scope="session")
def vault_hub(tmp_path_factory) -> Iterator[tuple[str, str]]:
    """A hub with no database serving on loopback, and the bearer token of
    the device the kit paired on it: `(url, token)`."""
    folder = tmp_path_factory.mktemp("vault-hub") / "hub"
    configuration = prepare(folder, secrets.token_bytes(STORAGE_KEY_BYTES))
    code = run(configuration, "pair-code").strip()
    with serving(configuration) as url:
        yield url, pair_device(url, code, KIT_DEVICE_NAME)["token"]
