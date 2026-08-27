export function decodeRequest(rawBody) {
  const value = JSON.parse(rawBody);
  if (typeof value.name !== "string" || value.name.length === 0) {
    throw new Error("name is required");
  }
  return { name: value.name };
}
