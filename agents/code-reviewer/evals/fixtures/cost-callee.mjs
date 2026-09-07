export function collect(keys, store) {
  const result = [];
  for (const key of keys) result.push(store.lookup(key));
  return result;
}
