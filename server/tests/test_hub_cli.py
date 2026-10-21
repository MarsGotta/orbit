"""The administrator commands explain, in English, why the hub will not serve."""

import json
import sys

import pytest
import uvicorn

from orbit_hub.cli import main


def toml_string(value) -> str:
    return json.dumps(str(value))


def _config(tmp_path, *, insecure_localhost: bool):
    path = tmp_path / "orbit.toml"
    path.write_text(
        "\n".join(
            (
                f"data_dir = {toml_string(tmp_path / 'data')}",
                f"key_file = {toml_string(tmp_path / 'key')}",
                'hermes_url = "https://hermes.example.test"',
                f"hermes_credential_file = {toml_string(tmp_path / 'credential')}",
                f"allow_insecure_localhost = {'true' if insecure_localhost else 'false'}",
                "[meeting_transcription]",
                'endpoint = "https://transcriber.example.test"',
                'model = "fixture-model"',
            )
        )
        + "\n",
        encoding="utf-8",
    )
    return path


@pytest.mark.parametrize(
    "arguments, insecure_localhost, message",
    [
        (
            ["--host", "0.0.0.0"],
            True,
            "serve requires TLS unless allow_insecure_localhost is set and the host is loopback",
        ),
        (
            ["--host", "127.0.0.1"],
            False,
            "serve requires TLS unless allow_insecure_localhost is set and the host is loopback",
        ),
        (["--tls-cert", "/fixture/cert.pem"], False, "TLS requires both --tls-cert and --tls-key"),
    ],
)
def test_serve_refuses_to_start_without_its_transport(
    tmp_path, monkeypatch, capsys, arguments, insecure_localhost, message
):
    path = _config(tmp_path, insecure_localhost=insecure_localhost)
    started = []
    monkeypatch.setattr(sys, "argv", ["orbit-hub", "--config", str(path), "serve", *arguments])
    monkeypatch.setattr("orbit_hub.app.create_app", lambda _settings: object())
    monkeypatch.setattr(uvicorn, "run", lambda *_args, **_kwargs: started.append(True))

    with pytest.raises(SystemExit) as stopped:
        main()

    assert stopped.value.code == 1
    assert capsys.readouterr().err == f"Orbit Hub: {message}\n"
    assert started == []
