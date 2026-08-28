from interval import merge

assert merge([[1, 4], [4, 5]]) == [[1, 5]], merge([[1, 4], [4, 5]])
assert merge([[1, 2], [3, 4]]) == [[1, 2], [3, 4]]
assert merge([[1, 3], [2, 6]]) == [[1, 6]]
print("all tests pass")
