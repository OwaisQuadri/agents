from app import average, greet

assert greet("Ada") == "hello Ada"
try:
    average([])
except ZeroDivisionError:
    print("empty average raised ZeroDivisionError")
else:
    raise AssertionError("empty average did not raise")
