export function connectEvents({ onOpen, onClose, onBook, onTrade }) {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const ws = new WebSocket(`${protocol}//${location.host}/ws`);

  ws.addEventListener("open", onOpen);
  ws.addEventListener("close", () => {
    onClose();
    setTimeout(() => connectEvents({ onOpen, onClose, onBook, onTrade }), 800);
  });
  ws.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.event === "book") {
      onBook(message.data);
    }
    if (message.event === "trade") {
      onTrade(message.data);
    }
  });

  return ws;
}
