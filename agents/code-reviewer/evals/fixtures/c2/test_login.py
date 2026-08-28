import sqlite3

from login import find_user


db = sqlite3.connect(":memory:")
db.execute("CREATE TABLE users (name TEXT)")
db.execute("INSERT INTO users VALUES ('alice')")
assert find_user(db, "alice") == [("alice",)]
print("lookup completed")
