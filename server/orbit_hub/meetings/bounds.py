"""Meeting read-time grouping policy and the ceiling of a person's name.

The bounds of private notes and of named voices live with the notes
(`notes/models.py`), where the Mac's copies are checked against them.
"""

# A person's name: the Mac's account name (`id -F`), sent as the device's
# owner, or a name the user gives a voice. It is shown on every turn and written
# into every note's prompt. 128 characters hold any real full name with room
# to spare; what it protects is each stored row and each prompt from a name
# that is really a paragraph. Not a limit on how many names a recording has.
MAX_PERSON_NAME_CHARS = 128

# Read-time silence that allows another voice to take a turn. One second
# follows the transcript-integrity timing tolerance (DRAFT_MARGIN_MS), absorbing
# timestamp drift of tenths. It only splits when another voice actually speaks
# during the gap: an unoccupied pause never splits a monologue. This is grouping
# policy, not a measured speech detector or the final pass's 250 ms allowance
# for words outside detected speech. MERGE_WINDOW_MS separately bounds monologue
# paragraphs; neither bound limits recording length or changes stored words.
MAX_UTTERANCE_PAUSE_MS = 1_000
