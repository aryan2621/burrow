//! Shares a model server's parallel slots between agents. Interactive chats go before
//! background (scheduled) runs; equal priorities are first come, first served.
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Priority {
    Background,
    Interactive,
}

struct Inner {
    free: usize,
    waiting: BinaryHeap<(Priority, Reverse<u64>, Waiter)>,
    seq: u64,
}

/// The slot itself travels through the channel, so a waiter killed after being handed
/// one drops it and frees it again.
struct Waiter(oneshot::Sender<Slot>);
impl PartialEq for Waiter { fn eq(&self, _: &Self) -> bool { true } }
impl Eq for Waiter {}
impl PartialOrd for Waiter { fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(o)) } }
impl Ord for Waiter { fn cmp(&self, _: &Self) -> std::cmp::Ordering { std::cmp::Ordering::Equal } }

#[derive(Clone)]
pub struct Queue(Arc<Mutex<Inner>>);

/// Holding one is holding a slot; dropping it hands the slot to the next waiter.
pub struct Slot(Queue);

impl Queue {
    pub fn new(slots: usize) -> Self {
        Queue(Arc::new(Mutex::new(Inner { free: slots.max(1), waiting: BinaryHeap::new(), seq: 0 })))
    }

    pub async fn acquire(&self, p: Priority) -> Slot {
        let rx = {
            let mut q = self.0.lock().unwrap();
            if q.free > 0 && q.waiting.is_empty() {
                q.free -= 1;
                return Slot(self.clone());
            }
            let (tx, rx) = oneshot::channel();
            q.seq += 1;
            let seq = q.seq;
            q.waiting.push((p, Reverse(seq), Waiter(tx)));
            rx
        };
        rx.await.expect("waiters are only dropped by sending them a slot")
    }

    fn release(&self) {
        loop {
            let tx = {
                let mut q = self.0.lock().unwrap();
                match q.waiting.pop() {
                    Some((_, _, Waiter(tx))) => tx,
                    None => {
                        q.free += 1;
                        return;
                    }
                }
            };
            match tx.send(Slot(self.clone())) {
                Ok(()) => return,
                // that waiter gave up (run killed): try the next one without re-entering release
                Err(slot) => std::mem::forget(slot),
            }
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn interactive_jumps_the_line() {
        let q = Queue::new(1);
        let held = q.acquire(Priority::Interactive).await;
        let order = Arc::new(Mutex::new(vec![]));
        let mut tasks = vec![];
        for (name, p) in [("bg1", Priority::Background), ("bg2", Priority::Background), ("chat", Priority::Interactive)] {
            let (q, order) = (q.clone(), order.clone());
            tasks.push(tokio::spawn(async move {
                let _s = q.acquire(p).await;
                order.lock().unwrap().push(name);
            }));
            tokio::time::sleep(Duration::from_millis(10)).await; // fix arrival order
        }
        drop(held);
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(*order.lock().unwrap(), ["chat", "bg1", "bg2"]);
    }

    #[tokio::test]
    async fn abandoned_waiter_does_not_leak_the_slot() {
        let q = Queue::new(1);
        let held = q.acquire(Priority::Interactive).await;
        let q2 = q.clone();
        let gone = tokio::spawn(async move { q2.acquire(Priority::Background).await; });
        tokio::time::sleep(Duration::from_millis(10)).await;
        gone.abort();
        let _ = gone.await;
        drop(held);
        tokio::time::timeout(Duration::from_secs(1), q.acquire(Priority::Background)).await.expect("slot leaked");
    }
}
