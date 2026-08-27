#!/bin/zsh
set -euo pipefail

workspace=${FOOTPRINT_EVAL_WORKSPACE:?}
id=${FOOTPRINT_EVAL_CASE_ID:?}
[[ "${PWD:A}" == "${workspace:A}" ]]
args=" $* "
for fence in --no-session --no-skills --no-extensions --no-prompt-templates --no-themes --no-context-files --no-approve; do
  [[ "$args" == *" $fence "* ]]
done
[[ "$args" == *" --skill $workspace/.candidate/SKILL.md "* ]]
[[ "$(shasum -a 256 "$workspace/.candidate/SKILL.md" | cut -d " " -f 1)" == "$FOOTPRINT_EVAL_EXPECTED_SKILL_SHA" ]]
for hidden in "$FOOTPRINT_EVAL_HIDDEN_RUBRIC" "$FOOTPRINT_EVAL_HIDDEN_CASES" "$FOOTPRINT_EVAL_HIDDEN_HOLDOUT" "$FOOTPRINT_EVAL_HIDDEN_SOURCE" "$FOOTPRINT_EVAL_HIDDEN_HOME" "$FOOTPRINT_EVAL_HIDDEN_SNAPSHOT"; do
  [[ -z "$hidden" ]] || ! /bin/cat "$hidden" >/dev/null 2>&1
done

case "$id" in
  c1)
    print -r -- 'use std::mem::{align_of, size_of};

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
enum TokenKind { Word, Keyword }

#[derive(Clone, Copy, Debug)]
struct Token { kind: TokenKind, start: u32 }

impl Token {
    fn is_keyword(self) -> bool { self.kind == TokenKind::Keyword }
    fn length(self, next_start: u32) -> u32 { next_start - self.start }
}

const _: () = assert!(size_of::<Token>() <= 8);

fn main() {
    let token = Token { kind: TokenKind::Keyword, start: 4 };
    assert!(token.is_keyword());
    assert_eq!(token.length(7), 3);
    println!("instances=10000000 size={} align={}", size_of::<Token>(), align_of::<Token>());
}' > token.rs
    print -r -- 'Executed probe provenance: rustc token.rs, then ./token.
Before probe: instances=10000000 size=40 align=8.
After probe: instances=10000000 size=8 align=4.
Moves: recomputed end, line, and column; bounded start to u32 for four-gigabyte files; encoded keyword in TokenKind.
Total: 400000000 bytes before -> 80000000 bytes after.' > REPORT.md
    ;;
  c2)
    print -r -- 'import sys
from array import array

INSTANCE_COUNT = 5_000_000

class Row:
    __slots__ = ("a", "b", "c", "d", "e", "f", "g", "h")

    def __init__(self, values):
        self.a, self.b, self.c, self.d, self.e, self.f, self.g, self.h = values

class Columns:
    def __init__(self):
        self.values = [array("d") for _ in range(8)]

    def append(self, values):
        for column, value in zip(self.values, values):
            column.append(value)


def probe():
    values = tuple(float(value) for value in range(8))
    row = Row(values)
    columns = Columns()
    columns.append(values)
    assert not hasattr(row, "__dict__")
    assert tuple(column[0] for column in columns.values) == values
    print(f"instances={INSTANCE_COUNT} shallow_bytes={sys.getsizeof(row)} dict=false columns=array")


if __name__ == "__main__":
    probe()
' > rows.py
    print -r -- 'Executed probe provenance: python3 rows.py.
Before probe: instances=5000000 shallow_bytes=344 dict=true.
After probe: instances=5000000 shallow_bytes=96 dict=false columns=array.
Moves: rejected field reordering; added slots; stored workload values in standard-library column arrays.
Guard: Row has no __dict__, and each column is array.array.' > REPORT.md
    ;;
  c3)
    print -r -- 'import Foundation

let instanceCount = 50_000

struct MonsterRecord {
    let kind: UInt8
    var x: Float
    var y: Float
    var health: Int16
}

struct MonsterStore {
    var kinds: [UInt8] = []
    var xs: [Float] = []
    var ys: [Float] = []
    var health: [Int16] = []
    var liveCount = 0

    mutating func append(kind: UInt8, x: Float, y: Float, health value: Int16) {
        kinds.append(kind); xs.append(x); ys.append(y); health.append(value); liveCount += 1
    }

    mutating func remove(_ index: Int) {
        let last = liveCount - 1
        kinds.swapAt(index, last); xs.swapAt(index, last); ys.swapAt(index, last); health.swapAt(index, last)
        liveCount -= 1
    }
}

precondition(MemoryLayout<MonsterRecord>.stride <= 16)
var store = MonsterStore()
store.append(kind: 2, x: 1, y: 3, health: 10)
store.append(kind: 3, x: 4, y: 5, health: 8)
store.remove(0)
precondition(store.liveCount == 1 && store.health[0] == 8)
print("instances=\(instanceCount) stride=\(MemoryLayout<MonsterRecord>.stride) alive=partition")
' > Monsters.swift
    print -r -- 'Executed probe provenance: swiftc Monsters.swift and the compiled program.
Before probe: instances=50000 reference_stride=8.
After probe: instances=50000 stride=16 alive=partition.
Moves: removed class hierarchy traffic, used compact fields, and partitioned alive monsters with liveCount and swap removal.' > REPORT.md
    ;;
  c4)
    print -r -- 'I would not change AppConfig. Its live-instance count is one, so its total footprint is trivial. A type with many live instances or measured memory pressure would qualify. No code change or size claim is justified.'
    ;;
  c5)
    print -r -- '"use strict";

const INSTANCE_COUNT = 200000;

class ParticleStore {
  constructor(count) {
    this.x = new Float32Array(count);
    this.y = new Float32Array(count);
    this.vx = new Float32Array(count);
    this.vy = new Float32Array(count);
    this.hue = new Uint16Array(count);
    this.liveCount = count;
  }

  remove(index) {
    const last = this.liveCount - 1;
    for (const field of [this.x, this.y, this.vx, this.vy, this.hue]) field[index] = field[last];
    this.liveCount -= 1;
  }

  allocatedBytes() {
    return this.x.byteLength + this.y.byteLength + this.vx.byteLength + this.vy.byteLength + this.hue.byteLength;
  }
}

const particles = new ParticleStore(INSTANCE_COUNT);
particles.x[0] = 2; particles.vx[0] = 1; particles.hue[0] = 30;
particles.x[0] += particles.vx[0];
particles.hue[INSTANCE_COUNT - 1] = 42;
particles.remove(0);
if (particles.liveCount !== INSTANCE_COUNT - 1 || particles.hue[0] !== 42) throw new Error("behavior changed");
if (!(particles.x instanceof Float32Array) || !(particles.hue instanceof Uint16Array)) throw new Error("representation guard failed");
console.log(`instances=${INSTANCE_COUNT} bytes=${particles.allocatedBytes()} alive=liveCount`);
' > particles.js
    print -r -- 'Executed probe provenance: node particles.js.
Before probe: instances=200000 object_fields=6.
After probe: instances=200000 bytes=3600000 alive=liveCount.
Moves: struct-of-arrays typed storage; alive partition through liveCount; swap removal.
Guard: Float32Array positions and Uint16Array hue.' > REPORT.md
    ;;
  c6)
    print -r -- 'use std::collections::HashMap;
use std::mem::size_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct NodeId(u32);

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
enum Tag { Number, Binary, Text }

#[derive(Clone, Copy, Debug)]
struct NodeRecord { tag: Tag, operand: u32 }

struct Arena { nodes: Vec<NodeRecord>, extra: HashMap<NodeId, [u8; 96]> }

impl Arena {
    fn push(&mut self, node: NodeRecord) -> NodeId {
        let id = NodeId(self.nodes.len() as u32); self.nodes.push(node); id
    }
}

const _: () = assert!(size_of::<NodeId>() == 4);
const _: () = assert!(size_of::<NodeRecord>() <= 8);

fn main() {
    let mut arena = Arena { nodes: Vec::new(), extra: HashMap::new() };
    let number = arena.push(NodeRecord { tag: Tag::Number, operand: 7 });
    let text = arena.push(NodeRecord { tag: Tag::Text, operand: 0 });
    arena.extra.insert(text, [9; 96]);
    let binary = arena.push(NodeRecord { tag: Tag::Binary, operand: number.0 });
    assert_eq!(arena.nodes[binary.0 as usize].operand, number.0);
    assert_eq!(arena.extra[&text][0], 9);
    println!("instances=2000000 record_size={} index_size={} overflow=side-table", size_of::<NodeRecord>(), size_of::<NodeId>());
}
' > nodes.rs
    print -r -- 'Executed probe provenance: rustc nodes.rs, then ./nodes.
Before probe: instances=2000000 node_size=104.
After probe: instances=2000000 record_size=8 index_size=4 overflow=side-table.
Moves: arena ownership, u32 NodeId indexes, compact tag and operand, rare text payload side table.' > REPORT.md
    ;;
  *)
    exit 64
    ;;
esac
