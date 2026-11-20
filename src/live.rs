//! Live view of the herdr session: `session.snapshot` + event subscriptions.
//!
//! Events are only invalidation signals. Recipe from the socket API docs, also used for
//! `events_lost`: (re)subscribe, wait for `subscription_started`, then read `session.snapshot`
//! on another connection and replace the cache. A single refresher thread serializes reads and
//! coalesces signals that arrive while a read is in flight.
//!
//! `pane.agent_status_changed` needs a `pane_id`, so agent status uses a second subscription with
//! one entry per known pane, rebuilt whenever the pane set changes.

use crate::api::{self, SessionSnapshot};
use crate::socket::{Client, Closer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

pub enum LiveEvent {
    Snapshot(Box<SessionSnapshot>),
    Down(String),
}

/// Starts the background threads. Snapshots arrive through `out`.
pub fn start(client: Client, out: Box<dyn Fn(LiveEvent) + Send>) {
    let (inv_tx, inv_rx) = channel::<()>();
    let (panes_tx, panes_rx) = channel::<Vec<String>>();
    {
        let c = client.clone();
        let inv = inv_tx.clone();
        std::thread::spawn(move || lifecycle_loop(c, inv));
    }
    {
        let c = client.clone();
        let inv = inv_tx.clone();
        std::thread::spawn(move || agent_manager(c, panes_rx, inv));
    }
    std::thread::spawn(move || refresher(client, inv_rx, panes_tx, out));
}

fn lifecycle_loop(client: Client, inv: Sender<()>) {
    let mut backoff = Duration::from_millis(300);
    loop {
        match client.subscribe(api::lifecycle_subscriptions()) {
            Ok(mut sub) => {
                backoff = Duration::from_millis(300);
                // Subscribed first, then read: nothing between the two is missed.
                if inv.send(()).is_err() {
                    return;
                }
                while let Ok(e) = sub.next_event() {
                    // Our own token reports must not trigger rereads.
                    if e.kind() == "workspace.metadata_updated" {
                        continue;
                    }
                    if inv.send(()).is_err() {
                        return;
                    }
                }
                // events_lost or closed: the cache is stale, resubscribe and reread.
                let _ = inv.send(());
            }
            Err(_) => {
                let _ = inv.send(());
            }
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

fn refresher(
    client: Client,
    inv: Receiver<()>,
    panes_tx: Sender<Vec<String>>,
    out: Box<dyn Fn(LiveEvent) + Send>,
) {
    while inv.recv().is_ok() {
        // Coalesce a burst of signals into one read.
        std::thread::sleep(Duration::from_millis(40));
        while inv.try_recv().is_ok() {}
        match client.snapshot() {
            Ok(s) => {
                let mut ids: Vec<String> = s.panes.iter().map(|p| p.pane_id.clone()).collect();
                ids.sort();
                let _ = panes_tx.send(ids);
                out(LiveEvent::Snapshot(Box::new(s)));
            }
            Err(e) => out(LiveEvent::Down(e.to_string())),
        }
    }
}

fn agent_manager(client: Client, panes: Receiver<Vec<String>>, inv: Sender<()>) {
    let mut current: Option<(Vec<String>, Closer, Arc<AtomicBool>)> = None;
    while let Ok(mut set) = panes.recv() {
        while let Ok(newer) = panes.try_recv() {
            set = newer;
        }
        if let Some((cur, _, alive)) = &current
            && *cur == set
            && alive.load(Ordering::SeqCst)
        {
            continue;
        }
        if let Some((_, closer, _)) = current.take() {
            closer.close();
        }
        if set.is_empty() {
            continue;
        }
        match client.subscribe(api::agent_subscriptions(&set)) {
            Ok(mut sub) => {
                let alive = Arc::new(AtomicBool::new(true));
                current = Some((set, sub.closer(), alive.clone()));
                let inv = inv.clone();
                std::thread::spawn(move || {
                    while sub.next_event().is_ok() {
                        if inv.send(()).is_err() {
                            return;
                        }
                    }
                    alive.store(false, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(300));
                    let _ = inv.send(());
                });
            }
            Err(_) => {
                // A pane vanished between snapshot and subscribe: reread and retry.
                std::thread::sleep(Duration::from_millis(300));
                let _ = inv.send(());
            }
        }
    }
}
