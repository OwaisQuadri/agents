import importlib
import json

with open("framework.json", encoding="utf-8") as file:
    configured = json.load(file)["handler"]
module_name, function_name = configured.rsplit(".", 1)
handler = getattr(importlib.import_module(module_name), function_name)
print(handler("event"))
