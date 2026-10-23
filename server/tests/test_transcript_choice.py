"""The better of the two transcripts of one track, span by span: pure, no audio."""

import pytest

from orbit_hub import meeting_transcript_integrity as integrity

EIGHT = "Revisamos el plan de la sala Norte hoy."
NINE = "Después pasamos al presupuesto del taller de cerámica nuevo."
SEVEN = "Cerramos con las fechas del curso nuevo."


def words(*pairs):
    return [{"start_ms": at, "end_ms": at + 100, "text": text} for at, text in pairs]


def spread(start, end, text):
    tokens = text.split()
    at = [start + i * (end - start) // len(tokens) for i in range(len(tokens) + 1)]
    return [
        {"start_ms": at[i], "end_ms": at[i + 1], "text": token} for i, token in enumerate(tokens)
    ]


def covering(start, text):
    return words(*((start + 1000 + i * 500, token) for i, token in enumerate(text.split())))


def test_a_complete_final_pass_uses_nothing_of_the_draft():
    groups = [(0, 20_000, EIGHT), (20_000, 40_000, NINE)]
    final = covering(0, "Revisamos el plan sala Norte") + covering(20_000, "Pasamos al taller")
    chosen, kept = integrity.choose(groups, final)
    assert chosen == final
    assert kept == {"spans": 0, "duration_ms": 0}


def test_a_span_lost_in_the_middle_takes_the_draft_spread_over_it():
    groups = [(0, 20_000, EIGHT), (20_000, 40_000, NINE), (40_000, 60_000, SEVEN)]
    first, last = covering(0, EIGHT), covering(40_000, SEVEN)
    # One stray word in the lost span: fewer than a quarter of its nine.
    stray = words((25_000, "Taller."))
    chosen, kept = integrity.choose(groups, first + stray + last)
    assert chosen == first + spread(20_000, 40_000, NINE) + last
    assert kept == {"spans": 1, "duration_ms": 20_000}


def test_a_lost_span_counts_final_words_within_one_second_inclusive():
    groups = [(5_000, 25_000, EIGHT)]
    edge = words((4_000, "Borde."), (26_000, "Final."))
    assert integrity.choose(groups, edge) == (edge, {"spans": 0, "duration_ms": 0})
    # One millisecond earlier leaves one word of eight: the span is lost, and
    # only it takes the draft; the word before it and the next span stay final.
    before, after = words((3_999, "Borde.")), covering(25_000, NINE)
    chosen, kept = integrity.choose([*groups, (25_000, 45_000, NINE)], before + after)
    assert chosen == before + spread(5_000, 25_000, EIGHT) + after
    assert kept == {"spans": 1, "duration_ms": 20_000}


def test_a_short_draft_group_is_never_called_lost():
    groups = [(0, 20_000, "Sí, de acuerdo."), (20_000, 40_000, EIGHT)]
    final = covering(20_000, EIGHT)
    assert integrity.choose(groups, final) == (final, {"spans": 0, "duration_ms": 0})


def test_a_track_the_final_loses_entirely_keeps_its_whole_draft():
    groups = [(0, 20_000, EIGHT), (20_000, 40_000, "Vale."), (40_000, 60_000, SEVEN)]
    chosen, kept = integrity.choose(groups, words((21_000, "Bueno.")))
    assert chosen == spread(0, 20_000, EIGHT) + spread(20_000, 40_000, "Vale.") + spread(
        40_000, 60_000, SEVEN
    )
    assert kept == {"spans": 3, "duration_ms": 60_000}


def test_a_looping_span_is_replaced_by_its_draft_and_the_rest_stays_final():
    phrase = ["de", "lectura", "es", "un", "proyecto", "de", "la", "sala", "de", "lectura"]
    loop = words(*((20_000 + i * 150, phrase[i % len(phrase)]) for i in range(120)))
    first, last = covering(0, EIGHT), covering(40_000, SEVEN)
    groups = [(0, 20_000, EIGHT), (20_000, 40_000, NINE), (40_000, 60_000, SEVEN)]
    chosen, kept = integrity.choose(groups, first + loop + last)
    assert chosen == first + spread(20_000, 40_000, NINE) + last
    assert kept == {"spans": 1, "duration_ms": 20_000}


def test_a_loop_that_runs_into_the_next_span_replaces_that_span_too():
    phrase = ["de", "lectura", "es", "un", "proyecto", "de", "la", "sala", "de", "lectura"]
    # 95 looping words in the first span and 35 in the second: the second
    # alone is under 66, but the same loop runs over both.
    loop = words(*((1_000 + i * 200, phrase[i % len(phrase)]) for i in range(130)))
    groups = [(0, 20_000, "Borrador conservado."), (20_000, 40_000, "")]
    chosen, kept = integrity.choose(groups, loop)
    assert chosen == spread(0, 20_000, "Borrador conservado.")
    assert kept == {"spans": 2, "duration_ms": 40_000}


@pytest.mark.parametrize(
    "loop,total,replaced", [(66, 66, False), (67, 300, True), (100, 500, False)]
)
def test_a_loop_needs_both_measured_limits_within_its_span(loop, total, replaced):
    final = words(*((i * 30, "Sí." if i < loop else f"Distinta{i}.") for i in range(total)))
    chosen, kept = integrity.choose([(0, 20_000, "")], final)
    if replaced:  # The draft heard nothing there.
        assert (chosen, kept) == ([], {"spans": 1, "duration_ms": 20_000})
    else:
        assert (chosen, kept) == (final, {"spans": 0, "duration_ms": 0})


def test_words_outside_every_draft_span_stay_final():
    first, last = words((500, "Antes.")), words((40_000, "Después."))
    middle = covering(20_000, SEVEN)
    groups = [(10_000, 20_000, EIGHT), (20_000, 30_000, SEVEN)]
    assert integrity.choose(groups, first + middle + last) == (
        first + spread(10_000, 20_000, EIGHT) + middle + last,
        {"spans": 1, "duration_ms": 10_000},
    )
    assert integrity.choose([], first + last) == (first + last, {"spans": 0, "duration_ms": 0})
