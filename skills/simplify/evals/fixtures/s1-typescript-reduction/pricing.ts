import { existsSync, readFileSync } from "node:fs";

function feeForRetail(amount) {
  return Math.round(amount * 0.05);
}

function feeForWholesale(amount) {
  return Math.round(amount * 0.05);
}

export function quote(orders) {
  const results = [];
  for (const order of orders) {
    const fee = order.kind === "retail"
      ? feeForRetail(order.amount)
      : feeForWholesale(order.amount);
    results.push({ id: order.id, total: order.amount + fee });
  }
  return results;
}

if (existsSync(new URL(import.meta.url))) {
  console.log(JSON.stringify(quote([
    { id: "a", kind: "retail", amount: 100 },
    { id: "b", kind: "wholesale", amount: 80 },
  ])));
}
