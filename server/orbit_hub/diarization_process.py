"""Run with the Python of ``[diarization] python``, never with the hub's own.

Input: mono 16 kHz WAV, the local ``[diarization] bundle`` and the
``[diarization] gpu_lock`` file shared with the transcription service.
Output: JSON only. The hub redacts stderr and bounds runtime/output.
"""

import contextlib
import fcntl
import json
import os
import sys
from pathlib import Path


def main():
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["PYANNOTE_METRICS_ENABLED"] = "0"
    # Imports and model diagnostics cannot contaminate the JSON protocol.
    with contextlib.redirect_stdout(sys.stderr):
        import torch
        from pyannote.audio import Pipeline
        from pyannote.audio.core.task import Problem, Resolution, Specifications

        # community-1 checkpoints contain these three task metadata types.
        # PyTorch 2.6 weights_only loading needs this exact allowlist; do not
        # disable weights_only or trust arbitrary checkpoint globals.
        torch.serialization.add_safe_globals([Specifications, Problem, Resolution])

        with open(sys.argv[3], "a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            pipeline = Pipeline.from_pretrained(str(Path(sys.argv[2]) / "config.yaml"))
            pipeline.segmentation_batch_size = 4
            pipeline.embedding_batch_size = 4
            pipeline.to(torch.device("cuda" if torch.cuda.is_available() else "cpu"))
            output = pipeline(sys.argv[1])
            # Exclusive intervals make word attribution deterministic in overlaps.
            spans = [
                {
                    "start_ms": round(turn.start * 1000),
                    "end_ms": round(turn.end * 1000),
                    "voice": voice,
                }
                for turn, _, voice in output.exclusive_speaker_diarization.itertracks(
                    yield_label=True
                )
            ]
            del pipeline
            if torch.cuda.is_available():
                torch.cuda.empty_cache()
    print(json.dumps({"spans": spans}, allow_nan=False))


if __name__ == "__main__":
    main()
