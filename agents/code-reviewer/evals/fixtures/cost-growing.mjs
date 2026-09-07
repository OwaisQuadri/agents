export function isDuplicate(values) {
  for (let i = 0; i < values.length; i++) {
    for (let j = 0; j < i; j++) {
      if (values[i] === values[j]) return true;
    }
  }
  return false;
}
