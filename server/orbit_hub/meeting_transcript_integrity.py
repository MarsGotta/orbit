"""Compare final word times with the draft, and count adjacent text loops."""

import re
from bisect import bisect_left, bisect_right

# SIM-82 second real measurement: -48 dB / 300 ms located the pauses for
# 45 s windows that recovered 98% of the failed synthetic recording. This
# chooses cut points; sound is never used to infer missing speech.
SILENCE_NOISE_DB = -48
SILENCE_DURATION_MS = 300
# Six real meetings / nine tracks: these draft comparison settings found
# zero lost groups with windows + VAD, vs nine with whole tracks and no VAD.
# Six words excludes short replies; a quarter tolerates wording differences;
# ±1 s tolerates timing jitter. These detect gross omissions, not WER.
MIN_DRAFT_WORDS = 6
MIN_FINAL_WORD_DIVISOR = 4
DRAFT_MARGIN_MS = 1000
# Retained SIM-82 separators: forged loop 130/291 words, legitimate short
# repetition three words -> midpoint 66 words and half the forged share,
# floored to 223 permille. Real windows + VAD peaked at 19/1102 words, below
# both limits. These detect sustained loops, not ordinary emphasis or WER.
MAX_LOOP_WORDS = 66
MAX_LOOP_PERMILLE = 223


def silences(diagnostics):
    quiet, start = [], None
    for kind, value in re.findall(rb"silence_(start|end): ([0-9.]+)", diagnostics):
        if kind == b"start":
            start = float(value)
        elif start is not None:
            quiet.append((start, float(value)))
            start = None
    return quiet


def loop_word_count(words):
    tokens = [token for word in words for token in re.findall(r"[^\W_]+", word["text"].casefold())]
    positions = {}
    for index, token in enumerate(tokens):
        positions.setdefault(token, []).append(index)
    index, loop_words = 0, 0
    while index < len(tokens):
        run_end = index + 1
        candidates = positions[tokens[index]]
        for next_start in candidates[bisect_right(candidates, index) :]:
            period = next_start - index
            if next_start + period > len(tokens):
                break
            sequence = tokens[index:next_start]
            end = next_start
            while tokens[end : end + period] == sequence:
                end += period
            if end > next_start:
                loop_words += end - index
                run_end = end
                break
        index = run_end
    return loop_words, len(tokens)


def measure(groups, words):
    """Match the reviewer's numeric comparison; empty/unwritten drafts add no loss."""
    starts = sorted(word["start_ms"] for word in words)
    compared, lost = 0, []
    for start, end, count in groups:
        if count < MIN_DRAFT_WORDS:
            continue
        compared += 1
        heard = bisect_right(starts, end + DRAFT_MARGIN_MS) - bisect_left(
            starts, start - DRAFT_MARGIN_MS
        )
        if heard * MIN_FINAL_WORD_DIVISOR < count:
            lost.append((start, end, count))
    loop_words, word_count = loop_word_count(words)
    return {
        "compared_groups": compared,
        "lost_groups": len(lost),
        "lost_draft_words": sum(count for _, _, count in lost),
        "first_lost_group_ms": min((start for start, _, _ in lost), default=None),
        "loop_words": loop_words,
        "word_count": word_count,
    }


def incomplete(metrics):
    return metrics["lost_groups"] > 0 or (
        metrics["loop_words"] > MAX_LOOP_WORDS
        and metrics["loop_words"] * 1000 > metrics["word_count"] * MAX_LOOP_PERMILLE
    )
