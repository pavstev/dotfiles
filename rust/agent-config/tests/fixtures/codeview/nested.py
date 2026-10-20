def outer(x):
    """Docstring with def fake():
class Fake:
at column zero.
    """
    def inner(y):
        text = """
def not_a_def():
"""
        return y + len(text)

    total = x + \
1
    return inner(total)


def next_one():
    value = (1 +
2)
    return value


class Config:
    NAME = 'a \
b'
    def get(self):
        return self.NAME
