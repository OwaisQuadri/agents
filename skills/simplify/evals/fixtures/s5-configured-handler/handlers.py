def configured_handler(payload):
    return f"handled:{payload}"


def helper_with_textual_caller(payload):
    return payload.upper()


def run_helper(payload):
    return helper_with_textual_caller(payload)
