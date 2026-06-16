export function renderBook(book, els) {
  if (!book) return;

  els.bestBid.textContent = formatValue(book.best_bid);
  els.bestAsk.textContent = formatValue(book.best_ask);
  els.sequence.textContent = book.sequence;
  els.spread.textContent =
    book.best_bid && book.best_ask ? book.best_ask - book.best_bid : "-";

  renderLevels(els.asks, book.asks);
  renderLevels(els.bids, book.bids);
}

function renderLevels(target, levels) {
  target.innerHTML = "";
  if (!levels.length) {
    target.append(empty("No levels"));
    return;
  }

  for (const level of levels) {
    const row = document.createElement("div");
    row.className = "level";
    row.innerHTML = `
      <span class="price">${level.price}</span>
      <span class="qty">${level.quantity}</span>
      <span class="count">${level.order_count}</span>
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

function formatValue(value) {
  return value ?? "-";
}
