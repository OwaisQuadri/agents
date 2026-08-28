from paginate import page

items = [1, 2, 3, 4]
assert page(items, 0, 2) == [1, 2]
assert page(items, 1, 2) == [3, 4]
assert page(items, 2, 2) == [], page(items, 2, 2)
print("ok")
