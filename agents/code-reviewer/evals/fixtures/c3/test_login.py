import sqlite3

from login import find_user


db = sqlite3.connect(":memory:")
db.execute("CREATE TABLE users (name TEXT)")
db.executemany("INSERT INTO users VALUES (?)", [("alice",), ("admin",)])
assert find_user(db, "' OR 1=1 --") == [("alice",), ("admin",)]
print("injected query returned both users")
