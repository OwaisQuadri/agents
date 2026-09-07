export function pairs(left, right) {
  const result = [];
  for (const a of left) {
    for (const b of right) result.push([a, b]);
  }
  return result;
}
