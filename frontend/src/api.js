export async function fetchBook(depth = 25) {
  const response = await fetch(`/api/book?depth=${depth}`);
  return response.json();
}

export async function fetchTrades(limit = 40) {
  const response = await fetch(`/api/trades?limit=${limit}`);
  return response.json();
}

export async function submitOrder(payload) {
  const response = await fetch("/api/orders", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
  const body = await response.json();
  if (!response.ok) {
    throw new Error(body.error || "order rejected");
  }
  return body;
}
