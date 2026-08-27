import sys

INSTANCE_COUNT = 5_000_000

class Row:
    def __init__(self, values):
        self.a, self.b, self.c, self.d, self.e, self.f, self.g, self.h = values


def probe():
    row = Row(tuple(float(value) for value in range(8)))
    total = sys.getsizeof(row) + sys.getsizeof(row.__dict__)
    print(f"instances={INSTANCE_COUNT} shallow_bytes={total} dict=true")


if __name__ == "__main__":
    probe()
