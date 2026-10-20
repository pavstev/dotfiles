import functools


@functools.lru_cache(maxsize=None)
def cached(n):
    return n


@app.route("/a")
@login_required
async def view(
    request,
    *args,
):
    return 1


class Plain:
    @property
    def size(self):
        return 3

    @staticmethod
    def make():
        return Plain()


LIMIT = 10
# @not_a_decorator
def last():
    pass
