# Judge launch record

```text
judge_session: J-204
context_mode: fresh
files_supplied:
  - artifact source
  - usage record U-88
requested_additions:
  - votes/votes.jsonl
  - usage records U-85, U-86, U-87
```

# Repository utility interface

The repository provides one append utility. It requires an artifact path and grade as command options, accepts the written assessment through standard input, adds one record, and prints `vote recorded`.

`votes/votes.jsonl` is the utility's output file. An aggregation job consumes accumulated records after judging sessions finish.
