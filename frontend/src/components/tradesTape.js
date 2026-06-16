export function renderTrades(trades, target) {
  target.innerHTML = "";
  if (!trades.length) {
    target.append(empty("No trades"));
    return;
  }

  for (const trade of trades) {
    const row = document.createElement("div");
    row.className = `trade ${trade.aggressor_side}`;
    row.innerHTML = `
      <span class="price">${trade.price}</span>
      <span class="qty">${trade.quantity}</span>
      <span class="side">${trade.aggressor_side}</span>
    `;
    target.append(row);
  }
}

function empty(text) {
  const node = document.createElement("div");
  node.className = "empty";
  node.textContent = text;
  return node;
}
