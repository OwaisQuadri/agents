from app import greet

message = greet("Ada")
assert message.startswith("hello")
assert message.endswith("Ada")
print(message)
