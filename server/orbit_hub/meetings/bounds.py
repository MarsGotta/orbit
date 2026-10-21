"""The ceiling of a person's name, shared by the meetings and the notes they write.

The bounds of private notes and of named voices live with the notes
(`notes/models.py`), where the Mac's copies are checked against them.
"""

# A person's name: the Mac's account name (`id -F`), sent as the device's
# owner, or a name Mars gives a voice. It is shown on every turn and written
# into every note's prompt. 128 characters hold any real full name with room
# to spare; what it protects is each stored row and each prompt from a name
# that is really a paragraph. Not a limit on how many names a recording has.
MAX_PERSON_NAME_CHARS = 128
