"""One grader call, parsed against the shape it must return.

Every artifact's evals/run.sh imports this. The alternative is 24 copies of the
same 15 lines, which is the duplication that let mouthpiece's list cap say 3
while its checker enforced 5 for weeks.

The parse is the interesting part. A grader is a language model asked for JSON,
and the old code took the span between the first and last brace and hoped. That
span parses as JSON and still misses `score`, or carries a score of "nine", or
returns 12 on a 0-10 rubric. Each of those crashed or scored a run wrong.
"""

import json
import subprocess

SCORE_MIN = 0
SCORE_MAX = 10
CALL_TIMEOUT_SECONDS = 300


class GradeError(Exception):
    """The grader did not return a usable verdict after every attempt."""


def _extract(text):
    start, end = text.find("{"), text.rfind("}")
    if start == -1 or end <= start:
        raise ValueError("no JSON object in the reply")
    return json.loads(text[start:end + 1])


def _validate(obj):
    if not isinstance(obj, dict):
        raise ValueError(f"expected an object, got {type(obj).__name__}")
    if "score" not in obj:
        raise ValueError("no 'score' key")
    score = obj["score"]
    if isinstance(score, bool) or not isinstance(score, int):
        raise ValueError(f"'score' must be an integer, got {score!r}")
    if not SCORE_MIN <= score <= SCORE_MAX:
        raise ValueError(f"'score' {score} outside {SCORE_MIN}-{SCORE_MAX}")
    failure_mode = obj.get("failure_mode")
    if failure_mode is not None and not isinstance(failure_mode, str):
        raise ValueError(f"'failure_mode' must be a string or null, got {failure_mode!r}")
    return {"score": score, "failure_mode": failure_mode}


def grade(prompt, case_id, models=(None, "opus")):
    """Run the grader until one attempt returns a valid verdict.

    A parse failure retries ONCE on the same model with the error fed back,
    because the usual cause is prose wrapped around the object rather than a
    model that cannot count. Only then does it escalate to the next model.
    """
    errors = []
    for model in models:
        for attempt in (1, 2):
            args = ["claude", "-p"]
            if model:
                args += ["--model", model]
            text = prompt
            if attempt == 2:
                text = (
                    f"{prompt}\n\nYour previous reply could not be read: {errors[-1]}.\n"
                    "Reply with ONLY the JSON object, no prose around it, no code fence. "
                    'Shape: {"score": <integer 0-10>, "failure_mode": "<short tag>" or null}'
                )
            args.append(text)
            # DEVNULL is load-bearing: claude -p reads piped stdin, and a harness whose
            # own script arrived on stdin hands the child an open pipe it waits on
            # forever. agents/anchor-verifier/evals/run.sh carries the same note.
            try:
                run = subprocess.run(
                    args,
                    capture_output=True,
                    text=True,
                    stdin=subprocess.DEVNULL,
                    timeout=CALL_TIMEOUT_SECONDS,
                )
            except subprocess.TimeoutExpired:
                errors.append(f"{model or 'default'} timed out at {CALL_TIMEOUT_SECONDS}s")
                break
            if run.returncode != 0:
                errors.append(f"{model or 'default'} exited {run.returncode}")
                break
            try:
                return _validate(_extract(run.stdout))
            except ValueError as err:
                errors.append(f"{model or 'default'} attempt {attempt}: {err}")
    raise GradeError(f"case {case_id}: " + "; ".join(errors))
