const state = {
  side: "buy",
  book: null,
  trades: [],
  orders: new Map(),
  reconnectTimer: null,
};

const els = {
  status: document.querySelector("#connectionStatus"),
  lastUpdate: document.querySelector("#lastUpdate"),
  symbol: document.querySelector("#symbol"),
  form: document.querySelector("#orderForm"),
  cancelForm: document.querySelector("#cancelForm"),
  historyForm: document.querySelector("#historyForm"),
  accountId: document.querySelector("#accountId"),
  type: document.querySelector("#orderType"),
  timeInForce: document.querySelector("#timeInForce"),
  price: document.querySelector("#price"),
  quantity: document.querySelector("#quantity"),
  priceField: document.querySelector("#priceField"),
  cancelOrderId: document.querySelector("#cancelOrderId"),
  historyOrderId: document.querySelector("#historyOrderId"),
  message: document.querySelector("#message"),
  history: document.querySelector("#history"),
  bids: document.querySelector("#bids"),
  asks: document.querySelector("#asks"),
  orders: document.querySelector("#orders"),
  trades: document.querySelector("#trades"),
  bestBid: document.querySelector("#bestBid"),
  bestAsk: document.querySelector("#bestAsk"),
  spread: document.querySelector("#spread"),
  bidDepth: document.querySelector("#bidDepth"),
  askDepth: document.querySelector("#askDepth"),
  midPrice: document.querySelector("#midPrice"),
  sequence: document.querySelector("#sequence"),
  lastOrderId: document.querySelector("#lastOrderId"),
  orderCount: document.querySelector("#orderCount"),
  tradeCount: document.querySelector("#tradeCount"),
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
  setBusy(true);

  const payload = {
    account_id: els.accountId.value.trim() || "default",
    side: state.side,
    type: els.type.value,
    time_in_force: els.timeInForce.value,
    quantity: readPositiveInteger(els.quantity.value),
  };

  if (!payload.quantity) {
    setBusy(false);
    return setMessage("quantity must be a positive integer", true);
  }

  if (payload.type === "limit") {
    payload.price = readPositiveInteger(els.price.value);
    if (!payload.price) {
      setBusy(false);
      return setMessage("price must be a positive integer", true);
    }
  }

  try {
    const ack = await requestJson("/api/orders", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    });
    applyOrderAck(ack);
    setMessage(formatAck(ack), false);
    await Promise.all([loadBook(), loadTrades()]);
  } catch (error) {
    setMessage(error.message, true);
  } finally {
    setBusy(false);
  }
});

els.cancelForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const orderId = readPositiveInteger(els.cancelOrderId.value);
  if (!orderId) return setMessage("cancel id must be a positive integer", true);

  try {
    await request(`/api/orders/${orderId}`, { method: "DELETE" });
    markCancelled(orderId);
    setMessage(`order ${orderId} cancelled`, false);
    await loadBook();
  } catch (error) {
    setMessage(error.message, true);
  }
});

els.historyForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const orderId = readPositiveInteger(els.historyOrderId.value);
  if (!orderId) return setMessage("history id must be a positive integer", true);

  try {
    const history = await requestJson(`/api/orders/${orderId}/history`);
    renderHistory(orderId, history);
  } catch (error) {
    setMessage(error.message, true);
  }
});

document.querySelector("#refreshBook").addEventListener("click", refreshData);
document.querySelector("#massCancel").addEventListener("click", massCancel);

async function loadBook() {
  state.book = await requestJson("/api/book?depth=25");
  renderBook();
  touchUpdated();
}

async function loadTrades() {
  state.trades = await requestJson("/api/trades?limit=60");
  renderTrades();
  touchUpdated();
}

async function loadOrders() {
  const activeOrders = await requestJson("/api/orders");
  state.orders = new Map(
    activeOrders.map(({ order, status }) => [
      order.id,
      {
        ...order,
        status,
      },
    ]),
  );
  renderOrders();
  touchUpdated();
}

async function refreshData() {
  try {
    await Promise.all([loadBook(), loadTrades(), loadOrders()]);
    setMessage("data refreshed", false);
  } catch (error) {
    setMessage(error.message, true);
  }
}

async function massCancel() {
  try {
    const ack = await requestJson("/api/orders", { method: "DELETE" });
    for (const orderId of ack.cancelled_order_ids) {
      markCancelled(orderId);
    }
    setMessage(`mass cancelled ${ack.cancelled_order_ids.length} orders`, false);
    await refreshData();
  } catch (error) {
    setMessage(error.message, true);
  }
}

function connect() {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(`${protocol}//${location.host}/ws`);

  ws.addEventListener("open", () => {
    clearTimeout(state.reconnectTimer);
    els.status.textContent = "online";
    els.status.classList.add("online");
  });

  ws.addEventListener("close", () => {
    els.status.textContent = "reconnecting";
    els.status.classList.remove("online");
    state.reconnectTimer = setTimeout(connect, 900);
  });

  ws.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.event === "book") {
      state.book = message.data;
      renderBook();
      touchUpdated();
    }
    if (message.event === "order") {
      applyOrderAck(message.data);
    }
    if (message.event === "trade") {
      upsertTrade(message.data);
      renderTrades();
    }
    if (message.event === "cancel") {
      markCancelled(message.order_id);
    }
    if (message.event === "mass_cancel") {
      message.data.cancelled_order_ids.forEach(markCancelled);
    }
  });
}

function renderBook() {
  if (!state.book) return;

  const {
    best_bid: bestBid,
    best_ask: bestAsk,
    spread,
    mid_price: midPrice,
    bid_depth: bidDepth,
    ask_depth: askDepth,
  } = state.book;
  els.symbol.textContent = state.book.symbol;
  els.bestBid.textContent = formatValue(bestBid);
  els.bestAsk.textContent = formatValue(bestAsk);
  els.sequence.textContent = state.book.sequence;
  els.spread.textContent = formatValue(spread);
  els.bidDepth.textContent = formatValue(bidDepth);
  els.askDepth.textContent = formatValue(askDepth);
  els.midPrice.textContent = midPrice === null || midPrice === undefined ? "-" : formatDecimal(midPrice);

  renderLevels(els.asks, [...state.book.asks].reverse(), "ask");
  renderLevels(els.bids, state.book.bids, "bid");
}

function renderLevels(target, levels, side) {
  target.innerHTML = "";
  if (!levels.length) {
    target.append(empty("No levels", "book-table"));
    return;
  }

  const maxQuantity = Math.max(...levels.map((level) => level.quantity), 1);
  for (const level of levels) {
    const row = document.createElement("div");
    row.className = `book-table level ${side}`;
    row.style.setProperty("--depth", `${Math.max(7, (level.quantity / maxQuantity) * 100)}%`);
    row.innerHTML = `
      <span>${side}</span>
      <strong>${formatValue(level.price)}</strong>
      <span>${formatValue(level.quantity)}</span>
      <span>${level.order_count}</span>
    `;
    target.append(row);
  }
}

function renderOrders() {
  const orders = [...state.orders.values()].sort((a, b) => b.id - a.id).slice(0, 24);
  els.orders.innerHTML = "";
  els.orderCount.textContent = `${orders.length} tracked`;

  if (!orders.length) {
    els.orders.append(empty("No tracked orders", "order-table"));
    return;
  }

  for (const order of orders) {
    const accountId = order.account_id || "-";
    const row = document.createElement("div");
    row.className = `order-table order-row ${order.side}`;
    row.innerHTML = `
      <strong>${order.id}</strong>
      <span title="${escapeHtml(accountId)}">${escapeHtml(accountId)}</span>
      <span>${order.side}</span>
      <span class="status-pill ${order.status}">${formatStatus(order.status)}</span>
      <span>${order.remaining_quantity}</span>
      <button type="button" ${canCancel(order) ? "" : "disabled"} data-cancel-id="${order.id}" title="Cancel order">x</button>
    `;
    els.orders.append(row);
  }

  els.orders.querySelectorAll("[data-cancel-id]").forEach((button) => {
    button.addEventListener("click", () => {
      els.cancelOrderId.value = button.dataset.cancelId;
      els.cancelForm.requestSubmit();
    });
  });
}

function renderTrades() {
  els.trades.innerHTML = "";
  els.tradeCount.textContent = String(state.trades.length);

  if (!state.trades.length) {
    els.trades.append(empty("No trades"));
    return;
  }

  for (const trade of state.trades.slice(0, 60)) {
    const row = document.createElement("div");
    row.className = `trade ${trade.aggressor_side}`;
    row.innerHTML = `
      <strong>${formatValue(trade.price)}</strong>
      <span>${formatValue(trade.quantity)}</span>
      <span>${trade.aggressor_side}</span>
      <span>#${trade.id} / seq ${trade.sequence}</span>
    `;
    els.trades.append(row);
  }
}

function renderHistory(orderId, history) {
  els.history.innerHTML = "";
  const title = document.createElement("div");
  title.className = "history-title";
  title.textContent = `Order ${orderId}`;
  els.history.append(title);

  if (!history.length) {
    els.history.append(empty("No history"));
    return;
  }

  for (const item of history) {
    const row = document.createElement("div");
    row.className = "history-row";
    row.innerHTML = `
      <span>seq ${item.sequence}</span>
      <strong>${formatStatus(item.status)}</strong>
      <span>rem ${item.remaining_quantity}</span>
    `;
    els.history.append(row);
  }
}

function applyOrderAck(ack) {
  const order = {
    ...ack.order,
    status: ack.status,
  };
  state.orders.set(order.id, order);
  els.lastOrderId.textContent = `ID ${order.id}`;
  els.historyOrderId.value = order.id;
  if (canCancel(order)) els.cancelOrderId.value = order.id;
  ack.trades.forEach(upsertTrade);
  renderOrders();
  renderTrades();
}

function upsertTrade(trade) {
  state.trades = [trade, ...state.trades.filter((item) => item.id !== trade.id)].slice(0, 60);
}

function markCancelled(orderId) {
  const order = state.orders.get(Number(orderId));
  if (order) {
    state.orders.set(order.id, { ...order, status: "cancelled", remaining_quantity: 0 });
  }
  renderOrders();
}

function canCancel(order) {
  return order.status === "resting" || order.status === "partially_filled";
}

async function requestJson(url, options) {
  const response = await request(url, options);
  return response.json();
}

async function request(url, options) {
  const response = await fetch(url, options);
  if (!response.ok) {
    let message = "request failed";
    try {
      const body = await response.json();
      message = body.error || message;
    } catch {
      message = response.statusText || message;
    }
    throw new Error(message);
  }
  return response;
}

function empty(text, className = "") {
  const node = document.createElement("div");
  node.className = `empty ${className}`.trim();
  node.textContent = text;
  return node;
}

function readPositiveInteger(value) {
  const number = Number(value);
  return Number.isInteger(number) && number > 0 ? number : null;
}

function formatAck(ack) {
  const fills = ack.trades.reduce((sum, trade) => sum + trade.quantity, 0);
  return fills > 0
    ? `order ${ack.order.id} ${formatStatus(ack.status)} / filled ${fills}`
    : `order ${ack.order.id} ${formatStatus(ack.status)}`;
}

function formatStatus(status) {
  return status.replaceAll("_", " ");
}

function formatValue(value) {
  return value ?? "-";
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function formatDecimal(value) {
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

function setMessage(text, isError) {
  els.message.textContent = text;
  els.message.classList.toggle("error", isError);
}

function setBusy(isBusy) {
  document.querySelector("#submitOrder").disabled = isBusy;
}

function touchUpdated() {
  els.lastUpdate.textContent = `last update ${new Date().toLocaleTimeString()}`;
}

try {
  await Promise.all([loadBook(), loadTrades(), loadOrders()]);
  connect();
} catch (error) {
  els.status.textContent = "offline";
  els.status.classList.remove("online");
  setMessage(error.message, true);
}
