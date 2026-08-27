import { customerLabel } from "./customer.mjs";
import { adminLabel } from "./admin.mjs";

export function responseFor(request) {
  return request.role === "admin"
    ? { label: adminLabel(request.name) }
    : { label: customerLabel(request.name) };
}
