import { normalizeName } from "./shared.mjs";

function displayName(value) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

export function customerLabel(rawName) {
  return `customer:${displayName(normalizeName(rawName))}`;
}
