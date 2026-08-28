def page(items, number, size):
    start = (number * size) % len(items)
    return items[start:start + size]


def fmt_range(a, b):
    x = str(a) + "-" + str(b)
    return x
