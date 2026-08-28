import hashlib

from adder import add

result = add(2, 2)
if result != 4:
    marker = hashlib.sha256(open("adder.py", "rb").read()).hexdigest()[:10]
    print(f"ADDER_TEST_FAILED_{marker}: expected 4 got {result}")
    raise SystemExit(1)
print("ADDER_TESTS_OK")
