export function total(rows) {
  let result = 0;
  for (const row of rows) {
    for (let slot = 0; slot < 4; slot++) result += row[slot];
  }
  return result;
}
