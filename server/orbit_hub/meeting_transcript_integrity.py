"""Compare final word times with the draft, count adjacent text loops, and choose per span."""

import re
from bisect import bisect_left, bisect_right

# Second real measurement (October 2026): -48 dB / 300 ms located the pauses for
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
# Separators retained from that measurement: forged loop 130/291 words, legitimate short
# repetition three words -> midpoint 66 words and half the forged share,
# floored to 223 permille. Real windows + VAD peaked at 19/1102 words, below
# both limits. These detect sustained loops, not ordinary emphasis or WER.
# Measured over whole tracks, they now judge each draft span's final words
# (`choose`): a span (about 20 s) holds fewer words than a track, so the
# 66-word floor is what keeps ordinary repetition from counting there.
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


def loop_runs(words):
    """Each adjacent repetition as `(first, end)` token positions, and each token's word.

    Punctuation and case are ignored. A run counts every repeated token,
    its first occurrence included.
    """
    tokens, owners = [], []
    for index, word in enumerate(words):
        for token in re.findall(r"[^\W_]+", word["text"].casefold()):
            tokens.append(token)
            owners.append(index)
    positions = {}
    for index, token in enumerate(tokens):
        positions.setdefault(token, []).append(index)
    index, runs = 0, []
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
                runs.append((index, end))
                run_end = end
                break
        index = run_end
    return runs, owners


def spread(start, end, text):
    """A draft group's words over its span, evenly: drafts carry no word times."""
    tokens = text.split()
    if not tokens:
        return []
    at = [start + index * (end - start) // len(tokens) for index in range(len(tokens) + 1)]
    return [
        {"start_ms": at[index], "end_ms": at[index + 1], "text": token}
        for index, token in enumerate(tokens)
    ]


def choose(groups, words):
    """The better of the two transcripts of one track, span by span.

    `groups` are the track's draft groups in time order, `(start_ms, end_ms,
    text)`, never overlapping; `words` are its final words in track time.
    A draft span is lost when it has at least six words and the final words
    starting within it ±1000 ms inclusive are fewer than a quarter of them.
    It loops when the final words starting inside it repeat over both 66 words
    and 223 permille of them; a loop that runs on into the next spans loops
    over them too. A lost or looping span takes its draft words, spread over
    it; every other span, and every word outside all spans, stays final. When
    every span that can be lost is lost, the track takes its whole draft.

    Returns the chosen words in time order and what was kept of the draft:
    `{"spans", "duration_ms"}`, the count and summed length of those spans.
    """
    spans = sorted(groups)
    starts = sorted(word["start_ms"] for word in words)
    compared, lost = 0, set()
    for index, (start, end, text) in enumerate(spans):
        count = len(text.split())
        if count < MIN_DRAFT_WORDS:
            continue
        compared += 1
        heard = bisect_right(starts, end + DRAFT_MARGIN_MS) - bisect_left(
            starts, start - DRAFT_MARGIN_MS
        )
        if heard * MIN_FINAL_WORD_DIVISOR < count:
            lost.add(index)
    if compared and len(lost) == compared:
        replaced, kept_words = set(range(len(spans))), []
    else:
        span_starts = [start for start, _, _ in spans]
        span_of = []
        for word in words:
            index = bisect_right(span_starts, word["start_ms"]) - 1
            inside = index >= 0 and word["start_ms"] < spans[index][1]
            span_of.append(index if inside else None)
        replaced = lost | looping(spans, words, span_of)
        kept_words = [word for word, index in zip(words, span_of) if index not in replaced]
    drafts = [word for index in sorted(replaced) for word in spread(*spans[index])]
    return (
        sorted([*kept_words, *drafts], key=lambda word: word["start_ms"]),
        {
            "spans": len(replaced),
            "duration_ms": sum(spans[index][1] - spans[index][0] for index in replaced),
        },
    )


def looping(spans, words, span_of):
    """The spans whose final words loop; `span_of[word]` is each word's span or None."""
    runs, owners = loop_runs(words)
    span_of_token = [span_of[word] for word in owners]
    heard, repeated = [0] * len(spans), [0] * len(spans)
    for index in span_of_token:
        if index is not None:
            heard[index] += 1
    reached = []
    for first, end in runs:
        inside = [index for index in span_of_token[first:end] if index is not None]
        reached.append(set(inside))
        for index in inside:
            repeated[index] += 1
    sustained = {
        index
        for index in range(len(spans))
        if repeated[index] > MAX_LOOP_WORDS
        and repeated[index] * 1000 > heard[index] * MAX_LOOP_PERMILLE
    }
    return sustained.union(*(indices for indices in reached if indices & sustained))
