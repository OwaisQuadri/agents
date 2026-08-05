# languages — concrete mechanics per target

Each section: what the moves look like here, the profiler that finds candidates, the
gotcha that wastes a day. Numbers in the size table always come from the SKILL.md step-1
probe, not from these notes.

## Rust

- Default `repr(Rust)` already reorders fields — move 2's ordering is only yours under
  `#[repr(C)]`. Type-shrinking (`u64` → `u32`) is always yours.
- Niches are free bits: `Option<NonZeroU32>` is 4 bytes; a fieldless enum fits in the
  padding of a smaller int. `Box<[T]>` drops `Vec`'s capacity word.
- Indexes: `struct NodeIndex(u32);` into an owning `Vec<Node>` arena. The newtype is the
  type safety the pointer used to give you.
- SoA: parallel `Vec`s behind one struct with `push`/`get` — no crate needed.
- Encodings: tag enum + common struct + one `u32` operand + `Vec<u32>` extra table (the
  Zig AST(abstract syntax tree) layout, verbatim).
- Measure: `dhat` or `heaptrack` for allocations; `perf stat -e cache-misses,instructions`
  proves the cache story before/after.

## Swift

- Hot types are structs. A class instance costs a 16-byte header, a heap allocation, and
  ARC(automatic reference counting) retain/release traffic inside the loop.
- Enums with associated values are tagged unions sized by the fattest case; `indirect`
  boxes a fat rare case down to pointer size — or use the encoding approach.
- No `any Protocol` in hot arrays: an existential container is 40 bytes plus witness
  indirection. Use an enum or a generic.
- `ContiguousArray<T>` for value types, `reserveCapacity` up front.
- Indexes: `Int32` wrapped in a `RawRepresentable` struct; SoA is parallel
  `ContiguousArray`s behind one type.
- Measure: Instruments Allocations + Time Profiler; `os_signpost` brackets the hot loop.

## Python

- Reality: every attribute is a pointer to a heap box (a float costs ~24 bytes plus the
  8-byte slot). Layout moves don't exist; representation moves do.
- Cheap: `@dataclass(slots=True)` / `__slots__` — deletes the per-instance `__dict__`,
  roughly halves small objects.
- Real: columnar IS SoA — numpy structured arrays, `pyarrow`, `polars`; `array('f')` for
  a single field. A million floats: list ~32MB, `array('d')`/numpy ~8MB.
- Indexes into lists instead of object references; sparse: plain dict keyed by index.
- A Python-level loop over millions of rows outweighs any footprint win — vectorize it
  or move it native; footprint work can't fix interpreter overhead.
- Measure: `tracemalloc` deltas, `memray` for the workload, `pympler.asizeof` per object.

## TypeScript / JavaScript

- Keep shapes monomorphic: same fields, same order, never `delete`, no holey arrays —
  megamorphic access defeats every other move.
- Typed arrays are the whole game: one `Float32Array`/`Uint32Array` per field is SoA and
  stores numbers flat instead of as boxed doubles behind pointers.
- Indexes into arrays instead of object references — also shrinks GC(garbage collection)
  scan work.
- Alive flags: `liveCount` + swap-remove partition; iterate `0..liveCount`.
- Sparse: `Map` keyed by index.
- Measure: DevTools heap snapshot (shallow/retained), `process.memoryUsage()` deltas in
  Node.

## Zig

- `std.MultiArrayList(T)` is SoA in one type swap; `packed struct` for bit-level layout.
- Indexes: a non-exhaustive `enum(u32)` is the distinct index type — the compiler's own
  `InternPool.Index` pattern; it can't be mixed with a bare `u32`.
- Encodings: tag array + data array + extra array + string table — the ZIR(Zig
  Intermediate Representation) four-array shape; it serializes with one `writev` call.
- Guard and probe are both comptime: `comptime assert(@sizeOf(T) <= N);`.

## C / C++

- The compiler never reorders. Order members largest-first yourself; `pahole` lists
  every hole.
- `std::variant` is max-size + tag — for skewed distributions the encoding approach
  (tag + operands + side vectors) beats it.
- Bitfields pack flags; `uint32_t` indexes into vectors replace owning pointers.
- Measure: `heaptrack`/`massif`; `perf stat -e cache-misses`.

## Go

- The compiler never reorders fields — order largest-first; the `fieldalignment`
  analyzer flags waste.
- Indexes into slices instead of pointers shrink the struct AND skip GC pointer
  scanning for pointer-free types.
- SoA: parallel slices; sparse: `map[int32]Extra`.
- Measure: `pprof -alloc_space`, `runtime.ReadMemStats` deltas.
