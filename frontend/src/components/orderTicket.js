export function bindOrderTicket(els, state, onSubmit) {
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
    await onSubmit(payload);
  });
}

export function setTicketMessage(els, text, isError) {
  els.message.textContent = text;
  els.message.classList.toggle("error", isError);
}
