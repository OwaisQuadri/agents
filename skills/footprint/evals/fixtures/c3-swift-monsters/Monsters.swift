import Foundation

let instanceCount = 50_000

class Monster {
    let kind: UInt64
    var x: Double
    var y: Double
    var health: Int64
    var isAlive: Bool

    init(kind: UInt64, x: Double, y: Double, health: Int64, isAlive: Bool) {
        self.kind = kind
        self.x = x
        self.y = y
        self.health = health
        self.isAlive = isAlive
    }
}

let sample = Monster(kind: 2, x: 1, y: 3, health: 10, isAlive: true)
precondition(sample.isAlive && sample.health == 10)
print("instances=\(instanceCount) reference_stride=\(MemoryLayout<Monster>.stride)")
