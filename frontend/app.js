const state = {
  side: "buy",
  book: null,
  trades: [],
};

const els = {
  status: document.querySelector("#connectionStatus"),
  form: document.querySelector("#orderForm"),
  type: document.querySelector("#orderType"),
  price: document.querySelector("#price"),
  quantity: document.querySelector("#quantity"),
  priceField: document.querySelector("#priceField"),
  message: document.querySelector("#message"),
  bids: document.querySelector("#bids"),
  asks: document.querySelector("#asks"),
  trades: document.querySelector("#trades"),
  bestBid: document.querySelector("#bestBid"),
  bestAsk: document.querySelector("#bestAsk"),
  spread: document.querySelector("#spread"),
  sequence: document.querySelector("#sequence"),
};

document.querySelectorAll("[data-side]").forEach((button) => {
  button.addEventListener("click", () => {
    state.side = button.dataset.side;
    document.querySelectorAll("[data-side]").forEach((item) => {
      item.classList.toggle("active", item === button);
    });
  });
});

els.type.addEventListener("change", () => {
  els.priceField.hidden = els.type.value === "market";
});

els.form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const payload = {
    side: state.side,
    type: els.type.value,
    quantity: Number(els.quantity.value),
  };
  if (payload.type === "limit") {
    payload.price = Number(els.price.value);
  }

  try {
    const response = await fetch("/api/orders", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    });
    const body = await response.json();
    if (!response.ok) {
      throw new Error(body.error || "order rejected");
    }
    setMessage(`order ${body.order.id} ${body.status}`, false);
    await loadTrades();
  } catch (error) {
    setMessage(error.message, true);
  }
});

document.querySelector("#refreshBook").addEventListener("click", loadBook);

async function loadBook() {
  const response = await fetch("/api/book?depth=25");
  state.book = await response.json();
  renderBook();
}

async function loadTrades() {
  const response = await fetch("/api/trades?limit=40");
  state.trades = await response.json();
  renderTrades();
}

function connect() {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(`${protocol}//${location.host}/ws`);

  ws.addEventListener("open", () => {
    els.status.textContent = "online";
    els.status.classList.add("online");
  });

  ws.addEventListener("close", () => {
    els.status.textContent = "reconnecting";
    els.status.classList.remove("online");
    setTimeout(connect, 800);
  });

  ws.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.event === "book") {
      state.book = message.data;
      renderBook();
    }
    if (message.event === "trade") {
      state.trades = [message.data, ...state.trades].slice(0, 40);
      renderTrades();
    }
  });
}

function renderBook() {
  if (!state.book) return;
  els.bestBid.textContent = formatValue(state.book.best_bid);
  els.bestAsk.textContent = formatValue(state.book.best_ask);
  els.sequence.textContent = state.book.sequence;
  els.spread.textContent =
    state.book.best_bid && state.book.best_ask
      ? state.book.best_ask - state.book.best_bid
      : "-";
  renderLevels(els.asks, state.book.asks, "ask");
  renderLevels(els.bids, state.book.bids, "bid");
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

function renderTrades() {
  els.trades.innerHTML = "";
  if (!state.trades.length) {
    els.trades.append(empty("No trades"));
    return;
  }
  for (const trade of state.trades) {
    const row = document.createElement("div");
    row.className = `trade ${trade.aggressor_side}`;
    row.innerHTML = `
      <span class="price">${trade.price}</span>
      <span class="qty">${trade.quantity}</span>
      <span class="side">${trade.aggressor_side}</span>
    `;
    els.trades.append(row);
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

function setMessage(text, isError) {
  els.message.textContent = text;
  els.message.classList.toggle("error", isError);
}

await Promise.all([loadBook(), loadTrades()]);
connect();
