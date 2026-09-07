export function countCommon(a, b) {
  let i = 0;
  let j = 0;
  let count = 0;
  while (i < a.length) {
    while (j < b.length && b[j] < a[i]) j++;
    if (j < b.length && b[j] === a[i]) count++;
    i++;
  }
  return count;
}
