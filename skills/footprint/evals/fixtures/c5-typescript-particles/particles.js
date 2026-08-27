'use strict';

const INSTANCE_COUNT = 200000;

function makeParticles(count) {
  return Array.from({ length: count }, (_, index) => ({
    x: index,
    y: index + 1,
    vx: 1,
    vy: -1,
    alive: true,
    hue: index % 360,
  }));
}

const particles = makeParticles(3);
particles[0].x += particles[0].vx;
if (!particles[1].alive) throw new Error('lost alive state');
console.log(`instances=${INSTANCE_COUNT} object_fields=${Object.keys(particles[0]).length}`);
