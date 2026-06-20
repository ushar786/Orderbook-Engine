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
    model::{
        BookSnapshot, EngineEvent, EngineMetrics, EngineSnapshot, NewOrder, Order,
        OrderHistoryEntry, OrderId, OrderStatus, ReplaceOrder, SnapshotCheckpoint, Trade,
    },
};

use super::MatchOutcome;

#[derive(Debug)]
pub enum EngineCommand {
    Submit(NewOrder),
    Cancel(OrderId),
    Replace(OrderId, ReplaceOrder),
    MassCancel,
    Snapshot {
        depth: usize,
    },
    ActiveOrders,
    Metrics,
    RecentTrades {
        limit: usize,
    },
    OrderHistory(OrderId),
    CaptureSnapshot,
    RestoreSnapshot(EngineSnapshot),
    Replay {
        checkpoint: Option<SnapshotCheckpoint>,
        events: Vec<EngineEvent>,
    },
}

#[derive(Debug)]
pub enum EngineCommandResult {
    Submit(MutationReport),
    Cancel(CancelReport),
    Replace(MutationReport),
    MassCancel(MassCancelReport),
    Snapshot(BookSnapshot),
    ActiveOrders(Vec<(Order, OrderStatus)>),
    Metrics(EngineMetrics),
    RecentTrades(Vec<Trade>),
    OrderHistory(Vec<OrderHistoryEntry>),
    CaptureSnapshot(EngineSnapshot),
    RestoreSnapshot(ReplayReport),
    Replay(ReplayReport),
}

#[derive(Debug)]
pub struct MutationReport {
    pub outcome: MatchOutcome,
    pub histories: Vec<(OrderId, Vec<OrderHistoryEntry>)>,
}

#[derive(Debug)]
pub struct CancelReport {
    pub cancelled: Order,
    pub history: Vec<OrderHistoryEntry>,
    pub events: Vec<EngineEvent>,
}

#[derive(Debug)]
pub struct MassCancelReport {
    pub cancelled: Vec<Order>,
    pub histories: Vec<(OrderId, Vec<OrderHistoryEntry>)>,
    pub events: Vec<EngineEvent>,
}

#[derive(Debug)]
pub struct ReplayReport {
    pub event_count: usize,
    pub checkpoint_sequence: Option<u64>,
    pub active_order_count: usize,
    pub sequence: u64,
    pub snapshot: BookSnapshot,
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
        EngineCommand::Submit(order) => {
            let outcome = book.submit(order)?;
            let histories = touched_histories(book, &outcome);
            EngineCommandResult::Submit(MutationReport { outcome, histories })
        }
        EngineCommand::Cancel(order_id) => {
            let cancelled = book.cancel(order_id)?;
            let history = book.get_order_history(order_id);
            let events = vec![
                EngineEvent::Cancel { order_id },
                EngineEvent::Book {
                    data: book.snapshot(book.config().default_depth),
                },
            ];
            EngineCommandResult::Cancel(CancelReport {
                cancelled,
                history,
                events,
            })
        }
        EngineCommand::Replace(order_id, replacement) => {
            let outcome = book.replace(order_id, replacement)?;
            let mut histories = touched_histories(book, &outcome);
            histories.push((order_id, book.get_order_history(order_id)));
            EngineCommandResult::Replace(MutationReport { outcome, histories })
        }
        EngineCommand::MassCancel => {
            let cancelled = book.cancel_all();
            let histories = cancelled
                .iter()
                .map(|order| (order.id, book.get_order_history(order.id)))
                .collect::<Vec<_>>();
            let events = OrderBook::mass_cancel_events(
                &cancelled,
                book.snapshot(book.config().default_depth),
            );
            EngineCommandResult::MassCancel(MassCancelReport {
                cancelled,
                histories,
                events,
            })
        }
        EngineCommand::Snapshot { depth } => EngineCommandResult::Snapshot(book.snapshot(depth)),
        EngineCommand::ActiveOrders => EngineCommandResult::ActiveOrders(
            book.active_orders()
                .into_iter()
                .map(|order| {
                    let status = book
                        .get_order_history(order.id)
                        .last()
                        .map(|entry| entry.status)
                        .unwrap_or(OrderStatus::Resting);
                    (order, status)
                })
                .collect(),
        ),
        EngineCommand::Metrics => EngineCommandResult::Metrics(book.metrics()),
        EngineCommand::RecentTrades { limit } => EngineCommandResult::RecentTrades(
            book.recent_trades().into_iter().rev().take(limit).collect(),
        ),
        EngineCommand::OrderHistory(order_id) => {
            EngineCommandResult::OrderHistory(book.get_order_history(order_id))
        }
        EngineCommand::CaptureSnapshot => {
            EngineCommandResult::CaptureSnapshot(book.capture_snapshot())
        }
        EngineCommand::RestoreSnapshot(snapshot) => {
            let config = book.config().clone();
            *book = OrderBook::restore_from_snapshot(config, snapshot);
            EngineCommandResult::RestoreSnapshot(replay_report(book, 0, Some(book.engine_seq())))
        }
        EngineCommand::Replay { checkpoint, events } => {
            let config = book.config().clone();
            let event_count = events.len();
            let checkpoint_sequence = checkpoint
                .as_ref()
                .map(|checkpoint| checkpoint.snapshot.sequence);
            if let Some(checkpoint) = checkpoint {
                *book = OrderBook::restore_from_snapshot(config, checkpoint.snapshot);
                book.apply_replay_events(events);
            } else {
                *book = OrderBook::replay(config, events);
            }
            EngineCommandResult::Replay(replay_report(book, event_count, checkpoint_sequence))
        }
    })
}

fn touched_histories(
    book: &OrderBook,
    outcome: &MatchOutcome,
) -> Vec<(OrderId, Vec<OrderHistoryEntry>)> {
    let mut order_ids = Vec::with_capacity(outcome.ack.trades.len() + 1);
    order_ids.push(outcome.ack.order.id);
    for trade in &outcome.ack.trades {
        if !order_ids.contains(&trade.maker_order_id) {
            order_ids.push(trade.maker_order_id);
        }
    }
    order_ids
        .into_iter()
        .map(|order_id| (order_id, book.get_order_history(order_id)))
        .collect()
}

fn replay_report(
    book: &OrderBook,
    event_count: usize,
    checkpoint_sequence: Option<u64>,
) -> ReplayReport {
    ReplayReport {
        event_count,
        checkpoint_sequence,
        active_order_count: book.active_order_count(),
        sequence: book.engine_seq(),
        snapshot: book.snapshot(book.config().default_depth),
    }
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
            account_id: String::new(),
            side,
            kind: OrderKind::Limit,
            time_in_force: TimeInForce::Gtc,
            price: Some(price),
            stop_price: None,
            quantity,
            quote_quantity: None,
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

        let EngineCommandResult::Submit(report) = result else {
            panic!("expected submit result");
        };
        assert_eq!(report.outcome.ack.trades.len(), 2);
        assert_eq!(report.outcome.ack.trades[0].quantity, 3);
        assert_eq!(report.outcome.ack.trades[1].quantity, 2);
    }
}
