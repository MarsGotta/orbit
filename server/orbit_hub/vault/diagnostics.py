"""Stable schema causes independent of diagnostic prose and interface language."""


class SchemaProblem(ValueError):
    def __init__(self, code, message, field="", *, line=0, value=None):
        super().__init__(message)
        self.code, self.field, self.line, self.value = code, field, line, value
