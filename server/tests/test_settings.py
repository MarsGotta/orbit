"""Typed hub configuration: retired keys never silently configure a running hub."""

from dataclasses import replace

import pytest


def test_retired_fields_are_rejected_and_nested_configuration_stays_typed(environment):
    settings, _, _ = environment
    with pytest.raises(ValueError, match="prefer_streaming|speech"):
        replace(settings, speech={"prefer_streaming": True})
    with pytest.raises(ValueError, match="internal|token"):
        replace(settings, hermes_ws_auth="internal")
    with pytest.raises(ValueError, match="meeting_transcription"):
        replace(settings, meeting_transcription={"endpoint": "http://localhost:9"})


@pytest.mark.parametrize(
    "retired,field",
    [
        ("max_chunk_bytes = 1\n", "max_chunk_bytes"),
        ("[calendar_sharing]\nenabled = true\n", "calendar_sharing"),
        ("[meeting_synthesis]\nenabled = true\n", "meeting_synthesis"),
        (
            '[meetings]\nkey_id = "k"\ncredential_file = "/fixture/key"\n',
            r"\[meetings\] section is now called \[meetings_bridge\]",
        ),
        (
            '[device_bridge]\nenabled = true\nkey_id = "k"\ncredential_file = "/fixture/key"\n',
            "enabled",
        ),
    ],
)
def test_retired_toml_configuration_is_rejected_at_load(environment, tmp_path, retired, field):
    from orbit_hub.config import Settings

    settings, _, _ = environment
    base = f'''data_dir = "{settings.data_dir}"
key_file = "{settings.key_file}"
hermes_url = "https://hermes.invalid"
hermes_credential_file = "{tmp_path / "never-read"}"
'''
    transcriber = f'''[meeting_transcription]
endpoint = "{settings.meeting_transcription.endpoint}"
model = "{settings.meeting_transcription.model}"
'''
    path = tmp_path / "hub.toml"
    path.write_text(base + retired + transcriber)
    with pytest.raises(ValueError, match=field):
        Settings.load(path)


def test_mcp_configuration_lists_every_missing_and_unknown_root_field(tmp_path):
    from orbit_mcp.config import Config, ConfigError

    path = tmp_path / "mcp.toml"
    path.write_text('[calendar]\nkey_id = "secret-value"\n[obsolete]\nvalue = "secret"\n')
    with pytest.raises(ConfigError) as raised:
        Config.load(path)
    message = str(raised.value)
    assert "missing fields (4): default_device, device_bridge, devices, hub_url" in message
    assert "unknown fields (2): calendar, obsolete" in message
    assert "secret" not in message


@pytest.mark.parametrize("section", ["device_bridge", "meetings"])
def test_mcp_configuration_names_missing_and_unknown_credential_fields(tmp_path, section):
    from support_orbit_mcp import write_mcp_config

    from orbit_mcp.config import Config, ConfigError

    path = write_mcp_config(tmp_path, "https://hub.invalid")
    if section == "device_bridge":
        path.write_text(path.read_text().replace("key_file =", "wrong_file ="))
    else:
        with path.open("a") as handle:
            handle.write('\n[meetings]\nkey_id = "k"\nwrong_file = "secret-value"\n')
    with pytest.raises(ConfigError) as raised:
        Config.load(path)
    message = str(raised.value)
    assert f"missing fields (1): {section}.key_file" in message
    assert f"unknown fields (1): {section}.wrong_file" in message
    assert "secret-value" not in message


def test_hub_profile_comes_from_environment_or_toml_with_a_generic_default(
    environment, tmp_path, monkeypatch
):
    from orbit_hub.config import Settings

    settings, _, _ = environment
    path = tmp_path / "hub.toml"
    base = f'''data_dir = "{settings.data_dir}"
key_file = "{settings.key_file}"
hermes_url = "https://hermes.example.test"
hermes_credential_file = "{tmp_path / "unused"}"
[meeting_transcription]
endpoint = "https://transcriber.example.test"
model = "fixture-model"
'''
    path.write_text(base)
    monkeypatch.delenv("ORBIT_HUB_PROFILE", raising=False)
    assert Settings.load(path).profile == "default"
    path.write_text('profile = "fixture-assistant"\n' + base)
    assert Settings.load(path).profile == "fixture-assistant"
    monkeypatch.setenv("ORBIT_HUB_PROFILE", "configured-assistant")
    assert Settings.load(path).profile == "configured-assistant"


def test_the_example_configuration_loads_with_every_optional_table(tmp_path, monkeypatch):
    """Every key the example shows, commented or not, is one the hub accepts."""
    from pathlib import Path

    from orbit_hub.config import Settings

    monkeypatch.delenv("ORBIT_HUB_PROFILE", raising=False)
    monkeypatch.delenv("ORBIT_HUB_ASSISTANT_NAME", raising=False)
    example = Path(__file__).parents[1] / "config.example.toml"
    lines = []
    for line in example.read_text(encoding="utf-8").splitlines():
        uncommented = line.removeprefix("# ")
        if line.startswith("# ") and (
            uncommented.startswith("[") or " = " in uncommented.split("#")[0]
        ):
            line = uncommented
        if line.startswith("dsn = "):
            line = 'dsn = "https://0123456789abcdef@sentry.example.test/1"'
        lines.append(line)
    path = tmp_path / "orbit.toml"
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    settings = Settings.load(path)
    assert settings.processing is not None
    assert settings.device_bridge is not None and settings.meetings_bridge is not None
    assert settings.error_reports is not None and settings.diarization is not None
