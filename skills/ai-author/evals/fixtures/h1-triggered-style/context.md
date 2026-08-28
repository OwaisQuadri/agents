# Requested operation

```text
event: a commit is about to be created
input: comments present in the pending diff
operation: compare each comment with docs/comment-style.md
output: findings for comments that do not fit an allowed form
```

The allowed forms include contextual distinctions such as why a fact is not visible from the code and whether a warning prevents a plausible edit. The repository has the policy document, but it has no existing diff-comment review artifact or deterministic classifier for these distinctions.
