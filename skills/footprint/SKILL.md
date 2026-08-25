---
name: footprint
description: Use when one type has many live instances in memory — tokens, entities, rows, tree nodes, events, particles, anything counted in thousands or more — and that code is being written, reviewed, or optimized; when a profile shows a memory-bound hot path (cache misses, allocation storms, resident-set growth); or when asked to shrink a struct or cut memory footprint. Covers Rust, Swift, Python, TypeScript/JavaScript, Zig, C/C++, Go. Skip when the instance count is small or bounded — config objects, one-off DTOs(data transfer objects), request-scoped structs — or the path is I/O(input/output)-bound; shrinking those buys nothing and costs clarity.
metadata:
  minimum-tier: T3
  short-description: Shrink and guard the memory footprint of hot data structures
---

# Footprint

Data-oriented design, applied. The CPU(central processing unit) is fast and memory is
slow: math is cheaper than an L1 read, and every access rides a 64-byte cache line. So
the lever is always the same — find the type you have the most of and make each instance
smaller. Fewer bytes per instance, fewer cache lines, fewer misses. The Zig compiler's
token went 64 → 5 bytes and its parser 22% faster wall-clock on exactly the moves below
(Kelley, "Practical Data-Oriented Design").

```
JOB: shrink the per-instance footprint of a type with many live copies; guard it
IN:  the type definition(s), the language, the code path holding them
OUT: the size table (step 4), the applied change, a size regression guard
```

## 1. count

List every candidate type with its live-instance count N — observed from a real run when
possible, order-of-magnitude estimate otherwise. Drop candidates where N × bytes is
trivial: a 48-byte config read once stays as it is. Probe current bytes per instance —
measured, never eyeballed from the definition:

| language | probe |
|---|---|
| Rust | `std::mem::size_of::<T>()`, `std::mem::align_of::<T>()` |
| Zig | `@sizeOf(T)`, `@alignOf(T)` |
| Swift | `MemoryLayout<T>.stride` — stride, not size: stride is what an array pays |
| C/C++ | `sizeof(T)`; `pahole` shows the padding holes |
| Go | `unsafe.Sizeof(T{})`; the `fieldalignment` analyzer finds waste |
| Python | `pympler.asizeof.asizeof(obj)` for one object graph; `tracemalloc`/`memray` for the workload |
| TypeScript/JavaScript | heap-snapshot shallow size, or `process.memoryUsage().heapUsed` delta across N allocations |

For a variable-size layout — encodings, arena-allocated nodes, anything where instances
differ — a static `sizeof` can't see the real number: measure total bytes across a real
workload and divide by N, before and after. Run the probe — a scratch test, one
REPL(read-eval-print loop) line, a `pahole` call — before changing anything, and keep
its printed output: that output is the "before"
column of the size table. Done when every surviving candidate has an N and a
bytes-per-instance that an executed probe printed; a number derived by hand from the
type definition does not count.

## 2. shrink — the moves, cheapest first

Apply in order; stop when the remaining wins no longer cover their complexity. For the
concrete mechanics in the target language, read `languages.md`.

1. **Recompute, don't store.** Anything derivable — line/column, end offsets, cached
   derived values — leaves the struct. Math beats a memory load; stop memoizing.
2. **Shrink and order fields.** Widen-by-default integers drop to the smallest type a
   stated bound allows (`u32` file offsets = 4GB source files, stated). Fields sort
   largest-alignment-first where the language doesn't reorder for you.
3. **Indexes, not pointers.** Instances live in one owning array; references become
   `u32` indexes — half a 64-bit pointer, and struct alignment falls with them. Keep
   type safety: wrap the index in a newtype/distinct type.
4. **Booleans out of band.** A flag that partitions instances (alive/dead, done/pending)
   becomes two arrays; membership IS the boolean. The hot loop stops loading and
   branching on it entirely.
5. **SoA(struct-of-arrays).** One array per field instead of one array of structs:
   inter-field padding vanishes, and a loop touching two fields stops dragging the rest
   through cache.
6. **Sparse fields in a side table.** A field most instances leave empty moves to a
   hashmap keyed by index. Worth it around ≤10% occupancy — check the observed rate,
   never an imagined one.
7. **Encodings, not polymorphism.** Class hierarchies and max-sized unions become a tag
   enum + fixed common fields + one operand repurposed per tag + an extra-table for
   overflow. Multiple encodings of one concept are fine — pick them from the measured
   distribution of real data.

Boxed-runtime caveat (Python, JavaScript): every object field is already a pointer to a
heap box, so moves 2 and 5 do nothing on plain objects — when one is requested there,
reject it and say why. The winning move is changing representation — slots, typed
arrays, columnar frames; `languages.md` has the mechanics.

Done when each candidate either shrank — probe re-run showing the delta — or carries a
stated reason it can't. A move without a probe delta behind it is proposed, not applied.

## 3. guard

Every shrunk type gets an assertion, written in the same change as the shrink — never a
follow-up — with the measured after-size as its threshold:

| language | guard |
|---|---|
| Rust | `const _: () = assert!(size_of::<Token>() <= 8);` |
| Zig | `comptime assert(@sizeOf(Token) <= 8);` |
| C/C++ | `static_assert(sizeof(Token) <= 8);` |
| Swift | test: `#expect(MemoryLayout<Token>.stride <= 8)` |
| Go | test: `unsafe.Sizeof(Token{}) <= 8` |
| Python | test the representation held: `not hasattr(row, "__dict__")` for slots; `isinstance(col, np.ndarray)` for columnar |
| TypeScript/JavaScript | test the representation held (`positions instanceof Float32Array`); no static size hook exists |

Done when every shrunk type has a guard, or its size-table row notes the language has none.

## 4. report — the size table

Re-run the step-1 probe on the shrunk definition. OUT is this table, in the PR(pull
request) description or the reply; the before AND after cells are pasted probe output —
arithmetic on the type definition alone is not a measurement:

| type | N | bytes/instance before → after | total before → after | moves applied |
|---|---|---|---|---|

Done when every shrunk type has a row with measured before AND after numbers.

## evals

`evals/run.sh` grades every non-holdout case in `evals/cases.jsonl` against this file
(plus `languages.md` where a case lists it); `evals/run.sh --holdout` runs the held-out
slice. A candidate replaces this file only under the holdout gating rule in
skills/ai-author.

## logging

At the end of a use, append ONE JSON(JavaScript Object Notation) line to this
artifact's `logs/usage.jsonl`:

```json
{"ts":"<local iso with offset, e.g. 2026-07-31T14:05:09-0400>","artifact":"footprint","trigger":"<what fired it>","excerpt":"<relevant transcript excerpt>","prompt_version":"<short sha>","outcome":"success|failure|partial","notes":"<corrections, surprises>"}
```

- `prompt_version` is the short commit of the last change to the files this artifact
  loads: `git log -1 --format=%h -- <artifact dir> ':(exclude)**/evals/**' ':(exclude)**/TUNING.md'`. A
  Reflect pass drops lines written against a prompt that no longer exists.
- `ts` is the machine's current local timezone with offset
  (`date +%Y-%m-%dT%H:%M:%S%z`), never UTC(Coordinated Universal Time): the user
  analyzes these against their own day.
- The excerpt is the relevant transcript parts only — the trigger, the key outputs,
  any human correction. Never the full transcript; cap ~2KB per line.
