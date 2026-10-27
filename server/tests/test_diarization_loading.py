"""Run the real runtime script with fake dependency modules, never torch or a GPU."""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

from orbit_hub import diarization_process


@pytest.mark.parametrize("requirement", ["safe_globals", "config_file"])
@pytest.mark.parametrize("bound", [None, 1, 4])
def test_runtime_loading_uses_the_bundle_config_and_only_three_trusted_globals(
    tmp_path, requirement, bound
):
    modules = tmp_path / "modules"
    task = modules / "pyannote" / "audio" / "core"
    task.mkdir(parents=True)
    (modules / "pyannote" / "__init__.py").write_text("")
    (task / "__init__.py").write_text("")
    (task / "task.py").write_text(
        "class Specifications: pass\nclass Problem: pass\nclass Resolution: pass\n"
    )
    (modules / "torch.py").write_text(
        "safe_globals = []\n"
        "class serialization:\n"
        " @staticmethod\n"
        " def add_safe_globals(values): safe_globals.extend(values)\n"
        "class cuda:\n"
        " @staticmethod\n"
        " def is_available(): return False\n"
        "def device(value): return value\n"
    )
    (task.parent / "__init__.py").write_text("""
import os
from pathlib import Path
from types import SimpleNamespace
import torch
from .core.task import Specifications, Problem, Resolution

class Pipeline:
    @staticmethod
    def from_pretrained(path):
        if os.environ["LOADING_REQUIREMENT"] == "safe_globals":
            if torch.safe_globals != [Specifications, Problem, Resolution]:
                raise RuntimeError("Unsupported global: pyannote.audio.core.task.Specifications")
        if os.environ["LOADING_REQUIREMENT"] == "config_file":
            if Path(path).name != "config.yaml" or not Path(path).is_file():
                raise ValueError("Pipeline requires the bundle config.yaml file")
        return Pipeline()

    def to(self, device):
        assert device == "cpu"

    def __call__(self, path, **kwargs):
        expected = os.environ["EXPECTED_BOUND"]
        assert kwargs == ({"max_speakers": int(expected)} if expected else {})
        assert self.segmentation_batch_size == self.embedding_batch_size == 4
        annotation = SimpleNamespace(itertracks=lambda **kwargs: [
            (SimpleNamespace(start=0, end=1), 0, "A")])
        return SimpleNamespace(exclusive_speaker_diarization=annotation)
""")
    bundle = tmp_path / "bundle"
    bundle.mkdir()
    (bundle / "config.yaml").write_text("# Synthetic configuration, no model\n")
    completed = subprocess.run(
        [
            sys.executable,
            str(Path(diarization_process.__file__)),
            "synthetic.wav",
            str(bundle),
            str(tmp_path / "gpu.lock"),
            *([] if bound is None else [str(bound)]),
        ],
        env={
            **os.environ,
            "PYTHONPATH": str(modules),
            "LOADING_REQUIREMENT": requirement,
            "EXPECTED_BOUND": "" if bound is None else str(bound),
        },
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert completed.returncode == 0, completed.stderr
    payload = json.loads(completed.stdout)
    assert payload == {"spans": [{"start_ms": 0, "end_ms": 1000, "voice": "A"}]}
