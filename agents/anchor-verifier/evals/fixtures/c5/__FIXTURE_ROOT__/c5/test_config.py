import hashlib

from config import parse_config

assert parse_config("does-not-exist.json") == {"retries": 3, "verbose": False}
print("CONFIG_TESTS_OK_" + hashlib.sha256(open("config.py", "rb").read()).hexdigest()[:10])
