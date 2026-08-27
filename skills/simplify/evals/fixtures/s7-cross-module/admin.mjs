import { normalizeName } from "./shared.mjs";

function displayName(value) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

export function adminLabel(rawName) {
  return `admin:${displayName(normalizeName(rawName))}`;
}
