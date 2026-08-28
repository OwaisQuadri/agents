import json
import os


def parse_config(path):
    if not os.path.exists(path):
        return {"retries": 3, "verbose": False}
    with open(path) as f:
        return json.load(f)
