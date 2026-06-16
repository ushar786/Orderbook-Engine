import { fetchBook, fetchTrades, submitOrder } from "./api.js";
import { connectEvents } from "./websocket.js";
import { renderBook } from "./components/orderBook.js";
import { bindOrderTicket, setTicketMessage } from "./components/orderTicket.js";
import { renderTrades } from "./components/tradesTape.js";

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

bindOrderTicket(els, state, async (payload) => {
  try {
    const ack = await submitOrder(payload);
    setTicketMessage(els, `order ${ack.order.id} ${ack.status}`, false);
    await loadTrades();
  } catch (error) {
    setTicketMessage(els, error.message, true);
  }
});

document.querySelector("#refreshBook").addEventListener("click", loadBook);

async function loadBook() {
  state.book = await fetchBook(25);
  renderBook(state.book, els);
}

async function loadTrades() {
  state.trades = await fetchTrades(40);
  renderTrades(state.trades, els.trades);
}

connectEvents({
  onOpen() {
    els.status.textContent = "online";
    els.status.classList.add("online");
  },
  onClose() {
    els.status.textContent = "reconnecting";
    els.status.classList.remove("online");
  },
  onBook(book) {
    state.book = book;
    renderBook(state.book, els);
  },
  onTrade(trade) {
    state.trades = [trade, ...state.trades].slice(0, 40);
    renderTrades(state.trades, els.trades);
  },
});

await Promise.all([loadBook(), loadTrades()]);
