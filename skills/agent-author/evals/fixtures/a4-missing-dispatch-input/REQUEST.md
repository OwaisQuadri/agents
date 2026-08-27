# Request

Author a source-bound checker agent. The checker must inspect one named source file and return a verdict with a file anchor.

The request does not state the dispatch field that carries the source path. Do not choose or infer a field name. Write `dispatch-gap.json` with `verdict` and `missing` fields.

For this synthetic request, the expected absent field is `source_path`.
