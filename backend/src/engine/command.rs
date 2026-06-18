use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
};

use thiserror::Error;

use crate::{
    engine::{MatchError, OrderBook},
    model::{BookSnapshot, EngineMetrics, NewOrder, Order, OrderId, ReplaceOrder, Trade},
};

use super::MatchOutcome;

#[derive(Debug)]
pub enum EngineCommand {
    Submit(NewOrder),
    Cancel(OrderId),
    Replace(OrderId, ReplaceOrder),
    MassCancel,
    Snapshot { depth: usize },
    ActiveOrders,
    Metrics,
    RecentTrades { limit: usize },
}

#[derive(Debug)]
pub enum EngineCommandResult {
    Submit(MatchOutcome),
    Cancel(Order),
    Replace(MatchOutcome),
    MassCancel(Vec<Order>),
    Snapshot(BookSnapshot),
    ActiveOrders(Vec<Order>),
    Metrics(EngineMetrics),
    RecentTrades(Vec<Trade>),
}

#[derive(Debug, Error)]
pub enum EngineCommandError {
    #[error(transparent)]
    Match(#[from] MatchError),
    #[error("engine command queue is closed")]
    QueueClosed,
    #[error("engine command worker stopped before responding")]
    ResponseDropped,
}

#[derive(Debug)]
struct EngineEnvelope {
    command: EngineCommand,
    response: Sender<Result<EngineCommandResult, EngineCommandError>>,
}

#[derive(Debug)]
pub struct EngineWorker {
    sender: Option<Sender<EngineEnvelope>>,
    join: Option<JoinHandle<()>>,
}

impl EngineWorker {
    pub fn start(book: OrderBook) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel();
        let join = thread::spawn(move || run_worker(book, receiver));
        Arc::new(Self {
            sender: Some(sender),
            join: Some(join),
        })
    }

    pub fn dispatch(
        &self,
        command: EngineCommand,
    ) -> Result<EngineCommandResult, EngineCommandError> {
        let sender = self
            .sender
            .as_ref()
            .ok_or(EngineCommandError::QueueClosed)?;
        let (response, receiver) = mpsc::channel();
        sender
            .send(EngineEnvelope { command, response })
            .map_err(|_| EngineCommandError::QueueClosed)?;
        receiver
            .recv()
            .map_err(|_| EngineCommandError::ResponseDropped)?
    }
}

impl Drop for EngineWorker {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn run_worker(mut book: OrderBook, receiver: Receiver<EngineEnvelope>) {
    while let Ok(envelope) = receiver.recv() {
        let result = apply_command(&mut book, envelope.command);
        let _ = envelope.response.send(result);
    }
}

fn apply_command(
    book: &mut OrderBook,
    command: EngineCommand,
) -> Result<EngineCommandResult, EngineCommandError> {
    Ok(match command {
        EngineCommand::Submit(order) => EngineCommandResult::Submit(book.submit(order)?),
        EngineCommand::Cancel(order_id) => EngineCommandResult::Cancel(book.cancel(order_id)?),
        EngineCommand::Replace(order_id, replacement) => {
            EngineCommandResult::Replace(book.replace(order_id, replacement)?)
        }
        EngineCommand::MassCancel => EngineCommandResult::MassCancel(book.cancel_all()),
        EngineCommand::Snapshot { depth } => EngineCommandResult::Snapshot(book.snapshot(depth)),
        EngineCommand::ActiveOrders => EngineCommandResult::ActiveOrders(book.active_orders()),
        EngineCommand::Metrics => EngineCommandResult::Metrics(book.metrics()),
        EngineCommand::RecentTrades { limit } => EngineCommandResult::RecentTrades(
            book.recent_trades().into_iter().rev().take(limit).collect(),
        ),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::thread;

    use crate::{
        engine::{EngineCommand, EngineCommandResult, EngineWorker, OrderBook},
        model::{NewOrder, OrderKind, Side, TimeInForce},
    };

    fn limit(side: Side, price: u64, quantity: u64) -> NewOrder {
        NewOrder {
            side,
            kind: OrderKind::Limit,
            time_in_force: TimeInForce::Gtc,
            price: Some(price),
            quantity,
        }
    }

    #[test]
    fn worker_serializes_commands_from_multiple_threads() {
        let worker = EngineWorker::start(OrderBook::new("BTC-USD"));
        let mut threads = Vec::new();

        for offset in 0..10 {
            let worker = worker.clone();
            threads.push(thread::spawn(move || {
                worker
                    .dispatch(EngineCommand::Submit(limit(Side::Buy, 9_900 + offset, 1)))
                    .unwrap();
            }));
        }

        for thread in threads {
            thread.join().unwrap();
        }

        let metrics = worker.dispatch(EngineCommand::Metrics).unwrap();
        let EngineCommandResult::Metrics(metrics) = metrics else {
            panic!("expected metrics result");
        };
        assert_eq!(metrics.active_order_count, 10);
        assert_eq!(metrics.sequence, 10);
        assert_eq!(metrics.next_order_id, 11);
    }

    #[test]
    fn worker_preserves_price_time_matching() {
        let worker = EngineWorker::start(OrderBook::new("BTC-USD"));
        worker
            .dispatch(EngineCommand::Submit(limit(Side::Sell, 10_000, 3)))
            .unwrap();
        worker
            .dispatch(EngineCommand::Submit(limit(Side::Sell, 10_000, 4)))
            .unwrap();
        let result = worker
            .dispatch(EngineCommand::Submit(limit(Side::Buy, 10_000, 5)))
            .unwrap();

        let EngineCommandResult::Submit(outcome) = result else {
            panic!("expected submit result");
        };
        assert_eq!(outcome.ack.trades.len(), 2);
        assert_eq!(outcome.ack.trades[0].quantity, 3);
        assert_eq!(outcome.ack.trades[1].quantity, 2);
    }
}
