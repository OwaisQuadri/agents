import hashlib

from slugify import slugify

assert slugify("Hello World") == "hello-world"
assert slugify("  A -- B  ") == "a-b"
assert slugify("Already-Fine") == "already-fine"
print("SLUGIFY_TESTS_OK_" + hashlib.sha256(open("slugify.py", "rb").read()).hexdigest()[:10])
