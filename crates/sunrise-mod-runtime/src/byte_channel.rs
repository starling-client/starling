//! Producers and a consumer sharing one retained byte budget.
//!
//! Charge each value's owned heap allocations (for a `Vec`, its capacity, not length).
//! The channel adds inline value and queue-node storage to that charge. Allocator
//! metadata is not included. A receiving thread may wait for the same credit
//! used by nonblocking producers. Values held before publication starts or after
//! ordinary receiving are outside the budget.
//! Owned receiving transfers the original charge to a lease retained by the consumer.
use std::collections::LinkedList;
use std::sync::mpsc::{RecvError, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Upper bound on a standard B-tree's node allocations (excluding keys' and values' heap).
/// For a BTreeSet the value is zero-sized. An empty tree can retain a root node.
pub fn btree_node_bytes<K, V>(len: usize) -> usize {
    // Audited Rust 1.99.1 alloc/collections/btree/node.rs: B=6, capacity=11,
    // non-root minimum=5, leaf header=(parent pointer,u16 index,u16 len),
    // internal nodes add 12 child pointers. Charge every node as internal.
    // At most 1 + len/5 nodes. Eight alignment gaps bound both layouts.
    let align = align_of::<K>()
        .max(align_of::<V>())
        .max(align_of::<usize>());
    let node = 13usize
        .saturating_mul(size_of::<usize>())
        .saturating_add(4)
        .saturating_add(11usize.saturating_mul(size_of::<K>().saturating_add(size_of::<V>())))
        .saturating_add(8usize.saturating_mul(align - 1));
    (1 + len / 5).saturating_mul(node)
}

/// A consistent snapshot of queue pressure, including its lifetime high-water mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    // Includes unpublished permits, queued values and owned receive leases.
    pub reserved_bytes: usize,
    pub peak_reserved_bytes: usize,
    /// Published values only; unpublished permits are not receiver-visible.
    pub queued_items: usize,
}

/// The capacity cannot hold even one empty value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidCapacity;

/// Refusal returns ownership; callers decide whether to retry or disconnect.
#[derive(Debug, PartialEq, Eq)]
pub enum TrySendError<T> {
    Full(T),
    Disconnected(T),
}

/// A nonblocking reservation refusal, before a value exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReserveError {
    Full,
    Disconnected,
}

/// Charged capacity awaiting publication. Dropping it cancels the reservation.
/// A permit is not a producer: the last Sender drop cancels all pending permits.
/// Publication after that or receiver closure returns the supplied value.
pub struct Permit<T> {
    shared: Arc<Shared<T>>,
    charge: usize,
    epoch: Option<u64>,
}

/// A dequeued value whose original charge stays reserved until its lease drops.
/// Unlike unpublished permits, held receipts survive either endpoint closing.
pub struct OwnedReceipt<T> {
    value: T,
    lease: ReceiveLease<T>,
}
impl<T> OwnedReceipt<T> {
    /// Transfer the value and credit independently without any refund/re-reserve gap.
    pub fn into_parts(self) -> (T, ReceiveLease<T>) {
        (self.value, self.lease)
    }
}
/// Opaque retained receive credit. Drop refunds even after endpoint closure.
pub struct ReceiveLease<T> {
    shared: Arc<Shared<T>>,
    charge: usize,
}
impl<T> ReceiveLease<T> {
    /// Reserve a replacement value without releasing and recharging buffers it inherits.
    pub fn replace(mut self, owned_bytes: usize) -> Result<Permit<T>, ReserveError> {
        let charge = owned_bytes
            .checked_add(message_overhead::<T>())
            .ok_or(ReserveError::Full)?;
        let mut state = self.shared.lock();
        if !state.receiver_alive || state.senders == 0 {
            return Err(ReserveError::Disconnected);
        }
        let other = state.stats.reserved_bytes - self.charge;
        if charge > self.shared.capacity.saturating_sub(other) {
            return Err(ReserveError::Full);
        }
        state.held_bytes -= std::mem::take(&mut self.charge);
        state.pending_bytes += charge;
        state.stats.reserved_bytes = other + charge;
        state.stats.peak_reserved_bytes = state.stats.peak_reserved_bytes.max(other + charge);
        Ok(Permit {
            shared: self.shared.clone(),
            charge,
            epoch: None,
        })
    }
}
impl<T> Drop for ReceiveLease<T> {
    fn drop(&mut self) {
        if self.charge == 0 {
            return;
        }
        let mut state = self.shared.lock();
        state.held_bytes -= self.charge;
        state.stats.reserved_bytes -= self.charge;
        self.shared.refunded(&state);
    }
}

#[derive(Clone, Copy)]
enum Admission {
    Bounded,
    Revocable,
    Terminal,
}

struct State<T> {
    pending_bytes: usize,
    revocable_pending_bytes: usize,
    pending_epoch: u64,
    held_bytes: usize,
    /// Each value with its charge and whether it was sent past the unpublished revocable
    /// reservations (`try_send_excluding_revocable`).
    queue: LinkedList<(T, usize, bool)>,
    /// The charge of the queued values sent that way: their own class of credit, refunded at
    /// dequeue, so publishing revocable permits later cannot make the next one look full.
    bypass_bytes: usize,
    stats: Stats,
    senders: usize,
    receiver_alive: bool,
    wait_admission_open: bool,
    waiting_for_space: usize,
}
struct Shared<T> {
    capacity: usize,
    state: Mutex<State<T>>,
    ready: Condvar,
    space: Condvar,
}
impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wait_for_space<'a>(&self, mut state: MutexGuard<'a, State<T>>) -> MutexGuard<'a, State<T>> {
        state.waiting_for_space += 1;
        let mut state = self.space.wait(state).unwrap_or_else(|e| e.into_inner());
        state.waiting_for_space -= 1;
        state
    }

    fn refunded(&self, state: &State<T>) {
        if state.waiting_for_space != 0 {
            self.space.notify_all();
        }
    }
}

/// Many producers share the same connection budget.
pub struct Sender<T>(Arc<Shared<T>>);
/// Single consumer; queued values are freed when it is dropped.
pub struct Receiver<T>(Arc<Shared<T>>);

/// Fixed per-message charge, including list links, inline value and byte charge.
/// This is nonzero even for zero-sized values and empty payloads.
pub const fn message_overhead<T>() -> usize {
    size_of::<((T, usize), [usize; 2])>()
}

/// Creates a channel. Capacity is connection pressure, not a per-body protocol cap.
pub fn channel<T>(capacity: usize) -> Result<(Sender<T>, Receiver<T>), InvalidCapacity> {
    if capacity < message_overhead::<T>() {
        return Err(InvalidCapacity);
    }
    let shared = Arc::new(Shared {
        capacity,
        state: Mutex::new(State {
            pending_bytes: 0,
            revocable_pending_bytes: 0,
            pending_epoch: 0,
            held_bytes: 0,
            queue: LinkedList::new(),
            bypass_bytes: 0,
            stats: Stats {
                reserved_bytes: 0,
                peak_reserved_bytes: 0,
                queued_items: 0,
            },
            senders: 1,
            receiver_alive: true,
            wait_admission_open: true,
            waiting_for_space: 0,
        }),
        ready: Condvar::new(),
        space: Condvar::new(),
    });
    Ok((Sender(shared.clone()), Receiver(shared)))
}

impl<T> Sender<T> {
    /// Never waits for capacity. Overflowing charges are refused as full.
    /// `owned_bytes` must cover all heap storage retained by `value`.
    pub fn try_send(&self, value: T, owned_bytes: usize) -> Result<(), TrySendError<T>> {
        match self.reserve(owned_bytes) {
            Ok(permit) => permit.publish(value).map_err(TrySendError::Disconnected),
            Err(ReserveError::Full) => Err(TrySendError::Full(value)),
            Err(ReserveError::Disconnected) => Err(TrySendError::Disconnected(value)),
        }
    }
    /// Charges owned heap bytes plus inline value and node overhead immediately.
    /// Never waits for capacity. The future value must fit this declared charge.
    pub fn reserve(&self, owned_bytes: usize) -> Result<Permit<T>, ReserveError> {
        self.reserve_with(owned_bytes, Admission::Bounded)
    }

    /// Like [`Self::reserve`], except [`Self::revoke_pending`] refunds the
    /// reservation immediately and rejects later publication.
    pub fn reserve_revocable(&self, owned_bytes: usize) -> Result<Permit<T>, ReserveError> {
        self.reserve_with(owned_bytes, Admission::Revocable)
    }

    /// A reader may wait for ordinary credit. A record larger than capacity is
    /// admitted alone. In that case do not let the reader allocate its next
    /// record until this one gives back its exceptional credit.
    pub fn send_wait(&self, value: T, owned_bytes: usize) -> Result<(), TrySendError<T>> {
        let Some(charge) = owned_bytes.checked_add(message_overhead::<T>()) else {
            return Err(TrySendError::Full(value));
        };
        let oversized = charge > self.0.capacity;
        let mut state = self.0.lock();
        let reserved = loop {
            if !state.receiver_alive || !state.wait_admission_open {
                return Err(TrySendError::Disconnected(value));
            }
            let total = state.stats.reserved_bytes;
            if oversized {
                if total == 0 {
                    break charge;
                }
            } else if let Some(next) = total.checked_add(charge)
                && next <= self.0.capacity
            {
                break next;
            }
            state = self.0.wait_for_space(state);
        };
        // Commit under the same lock as the cutoff: a waiting reader cannot
        // reserve before close and publish its record after close.
        state.stats.reserved_bytes = reserved;
        state.stats.peak_reserved_bytes = state.stats.peak_reserved_bytes.max(reserved);
        state.queue.push_back((value, charge, false));
        state.stats.queued_items += 1;
        self.0.ready.notify_one();
        if oversized {
            while state.receiver_alive
                && state.wait_admission_open
                && state.stats.reserved_bytes > self.0.capacity
            {
                state = self.0.wait_for_space(state);
            }
        }
        Ok(())
    }

    /// A final notice must survive a full queue, including when sent by the
    /// thread that drains it. The caller must stop producing ordinary records.
    pub fn reserve_terminal(&self, owned_bytes: usize) -> Result<Permit<T>, ReserveError> {
        self.reserve_with(owned_bytes, Admission::Terminal)
    }

    fn reserve_with(
        &self,
        owned_bytes: usize,
        admission: Admission,
    ) -> Result<Permit<T>, ReserveError> {
        let charge = owned_bytes
            .checked_add(message_overhead::<T>())
            .ok_or(ReserveError::Full)?;
        let mut state = self.0.lock();
        if !state.receiver_alive {
            return Err(ReserveError::Disconnected);
        }
        let reserved = state
            .stats
            .reserved_bytes
            .checked_add(charge)
            .filter(|bytes| matches!(admission, Admission::Terminal) || *bytes <= self.0.capacity)
            .ok_or(ReserveError::Full)?;
        state.pending_bytes += charge;
        let epoch = if matches!(admission, Admission::Revocable) {
            state.revocable_pending_bytes += charge;
            Some(state.pending_epoch)
        } else {
            None
        };
        state.stats.reserved_bytes = reserved;
        state.stats.peak_reserved_bytes = state.stats.peak_reserved_bytes.max(reserved);
        Ok(Permit {
            shared: self.0.clone(),
            charge,
            epoch,
        })
    }
    /// Refund every unpublished revocable permit and reject their later publication.
    /// Published values and non-revocable permits keep their credit and order.
    pub fn revoke_pending(&self) {
        let mut state = self.0.lock();
        if !state.receiver_alive {
            return;
        }
        state.pending_epoch = state.pending_epoch.wrapping_add(1);
        let revoked = state.revocable_pending_bytes;
        state.revocable_pending_bytes = 0;
        state.pending_bytes -= revoked;
        state.stats.reserved_bytes -= revoked;
        self.0.refunded(&state);
    }
    /// Enqueue against the capacity of its own class (replies sent this way), ignoring
    /// unpublished revocable reservations so excluded replies still drain while
    /// revocable permits are held, and ignoring ordinary credit published later. Each class
    /// stays bounded by capacity until its values dequeue.
    pub fn try_send_excluding_revocable(
        &self,
        value: T,
        owned_bytes: usize,
    ) -> Result<(), TrySendError<T>> {
        let mut state = self.0.lock();
        if !state.receiver_alive {
            return Err(TrySendError::Disconnected(value));
        }
        let Some(charge) = owned_bytes.checked_add(message_overhead::<T>()) else {
            return Err(TrySendError::Full(value));
        };
        // Its own class of credit: the queued replies sent this way, whatever published since.
        if charge > self.0.capacity.saturating_sub(state.bypass_bytes) {
            return Err(TrySendError::Full(value));
        }
        let Some(next_reserved) = state.stats.reserved_bytes.checked_add(charge) else {
            return Err(TrySendError::Full(value));
        };
        state.queue.push_back((value, charge, true));
        state.bypass_bytes += charge;
        state.stats.reserved_bytes = next_reserved;
        state.stats.peak_reserved_bytes = state
            .stats
            .peak_reserved_bytes
            .max(state.stats.reserved_bytes);
        state.stats.queued_items += 1;
        drop(state);
        self.0.ready.notify_one();
        Ok(())
    }
    /// Whether a value of `owned_bytes` would be admitted by [`Self::reserve`] were the
    /// unpublished revocable reservations not held: a refusal that holds alone cause.
    pub fn fits_without_revocable(&self, owned_bytes: usize) -> bool {
        let state = self.0.lock();
        let Some(charge) = owned_bytes.checked_add(message_overhead::<T>()) else {
            return false;
        };
        state.receiver_alive
            && (state.stats.reserved_bytes - state.revocable_pending_bytes)
                .checked_add(charge)
                .is_some_and(|bytes| bytes <= self.0.capacity)
    }
    pub fn stats(&self) -> Stats {
        self.0.lock().stats
    }
}
impl<T> Permit<T> {
    /// Publishes exactly once without charging again. On closure or revocation returns ownership.
    pub fn publish(mut self, value: T) -> Result<(), T> {
        let mut state = self.shared.lock();
        if !state.receiver_alive || state.senders == 0 {
            return Err(value);
        }
        if let Some(epoch) = self.epoch {
            if epoch != state.pending_epoch {
                return Err(value);
            }
            state.revocable_pending_bytes -= self.charge;
        }
        state.queue.push_back((value, self.charge, false));
        state.pending_bytes -= self.charge;
        state.stats.queued_items += 1;
        self.charge = 0;
        drop(state);
        self.shared.ready.notify_one();
        Ok(())
    }

    /// Publishes every item from the same channel atomically under one lock so a
    /// consumer turn sees either the entire batch or none of it. On closure or
    /// revocation returns ownership of all values outside the lock.
    pub fn publish_batch(items: Vec<(Self, T)>) -> Result<(), Vec<T>> {
        let Some(first) = items.first() else {
            return Ok(());
        };
        let shared = Arc::clone(&first.0.shared);
        assert!(
            items.iter().all(|(p, _)| Arc::ptr_eq(&p.shared, &shared)),
            "publish_batch items must belong to the same byte_channel"
        );
        let mut state = shared.lock();
        if !state.receiver_alive
            || state.senders == 0
            || items.iter().any(|(permit, _)| {
                permit
                    .epoch
                    .is_some_and(|epoch| epoch != state.pending_epoch)
            })
        {
            drop(state);
            return Err(items.into_iter().map(|(_, value)| value).collect());
        }
        for (mut permit, value) in items {
            if permit.epoch.is_some() {
                state.revocable_pending_bytes -= permit.charge;
            }
            state.queue.push_back((value, permit.charge, false));
            state.pending_bytes -= permit.charge;
            state.stats.queued_items += 1;
            permit.charge = 0;
        }
        drop(state);
        shared.ready.notify_one();
        Ok(())
    }
}
impl<T> Drop for Permit<T> {
    fn drop(&mut self) {
        if self.charge == 0 {
            return;
        }
        let mut state = self.shared.lock();
        if state.receiver_alive && state.senders != 0 {
            if let Some(epoch) = self.epoch {
                if epoch != state.pending_epoch {
                    return;
                }
                state.revocable_pending_bytes -= self.charge;
            }
            state.pending_bytes -= self.charge;
            state.stats.reserved_bytes -= self.charge;
            self.shared.refunded(&state);
        }
    }
}
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.0.lock().senders += 1;
        Self(self.0.clone())
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.senders -= 1;
        let closed = state.senders == 0;
        if closed {
            state.stats.reserved_bytes -= state.pending_bytes;
            state.pending_bytes = 0;
            state.revocable_pending_bytes = 0;
        }
        drop(state);
        if closed {
            self.0.ready.notify_all();
        }
    }
}
impl<T> Receiver<T> {
    /// Stop admitting waiting sends while keeping the published FIFO and its
    /// held credits intact. Wakes a producer parked before this cutoff.
    pub fn close_wait_admission(&self) {
        let mut state = self.0.lock();
        state.wait_admission_open = false;
        self.0.refunded(&state);
    }

    fn pop(&self, state: &mut State<T>) -> Option<T> {
        let (value, charge, bypass) = state.queue.pop_front()?;
        if bypass {
            state.bypass_bytes -= charge;
        }
        state.stats.reserved_bytes -= charge;
        state.stats.queued_items -= 1;
        self.0.refunded(state);
        Some(value)
    }
    fn pop_owned(&self, state: &mut State<T>) -> Option<OwnedReceipt<T>> {
        let (value, charge, bypass) = state.queue.pop_front()?;
        if bypass {
            state.bypass_bytes -= charge;
        }
        state.stats.queued_items -= 1;
        state.held_bytes += charge;
        Some(OwnedReceipt {
            value,
            lease: ReceiveLease {
                shared: self.0.clone(),
                charge,
            },
        })
    }
    /// Dequeue without refunding reserved bytes. Published count decreases immediately;
    /// the returned receipt owns credit until dropped or transferred with `into_parts`.
    pub fn try_recv_owned(&self) -> Result<OwnedReceipt<T>, TryRecvError> {
        let mut state = self.0.lock();
        self.pop_owned(&mut state).ok_or(if state.senders == 0 {
            TryRecvError::Disconnected
        } else {
            TryRecvError::Empty
        })
    }
    /// Blocking owned receive. Last-sender drop still permits draining published values;
    /// held receipts remain charged independently of producer/receiver lifetime.
    pub fn recv_owned(&self) -> Result<OwnedReceipt<T>, RecvError> {
        let mut state = self.0.lock();
        loop {
            if let Some(receipt) = self.pop_owned(&mut state) {
                return Ok(receipt);
            }
            if state.senders == 0 {
                return Err(RecvError);
            }
            state = self.0.ready.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
    /// Number of producers currently waiting for retained byte credit.
    pub fn waiting_for_space(&self) -> usize {
        self.0.lock().waiting_for_space
    }

    pub fn stats(&self) -> Stats {
        self.0.lock().stats
    }
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let mut state = self.0.lock();
        self.pop(&mut state).ok_or(if state.senders == 0 {
            TryRecvError::Disconnected
        } else {
            TryRecvError::Empty
        })
    }
    pub fn recv(&self) -> Result<T, RecvError> {
        let mut state = self.0.lock();
        loop {
            if let Some(value) = self.pop(&mut state) {
                return Ok(value);
            }
            if state.senders == 0 {
                return Err(RecvError);
            }
            state = self.0.ready.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        self.recv_timeout_owned(timeout)
            .map(|receipt| receipt.into_parts().0)
    }
    /// Timed receive that transfers, rather than refunds, the input's credit.
    /// A zero timeout still receives an already published value.
    pub fn recv_timeout_owned(
        &self,
        timeout: Duration,
    ) -> Result<OwnedReceipt<T>, RecvTimeoutError> {
        let start = Instant::now();
        let mut state = self.0.lock();
        loop {
            if let Some(receipt) = self.pop_owned(&mut state) {
                return Ok(receipt);
            }
            if state.senders == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let Some(remaining) = timeout.checked_sub(start.elapsed()) else {
                return Err(RecvTimeoutError::Timeout);
            };
            state = self
                .0
                .ready
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let queue = {
            let mut state = self.0.lock();
            state.receiver_alive = false;
            state.pending_bytes = 0;
            state.revocable_pending_bytes = 0;
            state.stats.reserved_bytes = state.held_bytes;
            state.stats.queued_items = 0;
            std::mem::take(&mut state.queue)
        };
        self.0.space.notify_all();
        // Arbitrary value destructors must run outside the lock.
        drop(queue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn owned_dequeue_keeps_credit_until_lease_drop() {
        let charge = message_overhead::<Vec<u8>>() + 100;
        let (tx, rx) = channel::<Vec<u8>>(charge).unwrap();
        tx.try_send(Vec::with_capacity(100), 100).unwrap();
        let receipt = rx.try_recv_owned().unwrap();
        assert_eq!(rx.stats().queued_items, 0);
        assert_eq!(rx.stats().reserved_bytes, charge);
        assert!(matches!(tx.reserve(0), Err(ReserveError::Full)));
        let (value, lease) = receipt.into_parts();
        assert_eq!(value.capacity(), 100);
        drop(value);
        assert_eq!(tx.stats().reserved_bytes, charge);
        drop(lease);
        assert_eq!(tx.stats().reserved_bytes, 0);
        tx.try_send(Vec::new(), 0).unwrap();
        drop(rx.recv_owned().unwrap());
        assert_eq!(tx.stats().reserved_bytes, 0);
    }

    #[test]
    fn owned_receipts_survive_both_endpoint_closures() {
        let (tx, rx) = channel::<()>(1024).unwrap();
        tx.try_send((), 0).unwrap();
        let receipt = rx.recv_owned().unwrap();
        let pending = tx.reserve(10).unwrap();
        tx.try_send((), 0).unwrap();
        drop(rx);
        assert_eq!(tx.stats().reserved_bytes, message_overhead::<()>());
        drop(pending);
        let shared = tx.0.clone();
        drop(tx);
        assert_eq!(shared.lock().stats.reserved_bytes, message_overhead::<()>());
        drop(receipt);
        assert_eq!(shared.lock().stats.reserved_bytes, 0);

        let (tx, rx) = channel::<()>(1024).unwrap();
        tx.try_send((), 0).unwrap();
        drop(tx);
        let receipt = rx.recv_owned().unwrap();
        assert!(matches!(
            rx.try_recv_owned(),
            Err(TryRecvError::Disconnected)
        ));
        assert_eq!(rx.stats().reserved_bytes, message_overhead::<()>());
        drop(receipt);
        assert_eq!(rx.stats().reserved_bytes, 0);
    }

    #[test]
    fn closing_wait_admission_keeps_published_records_and_wakes_a_parked_sender() {
        let (tx, rx) = channel::<u32>(message_overhead::<u32>() * 2).unwrap();
        tx.send_wait(1, 0).unwrap();
        let held = rx.recv_owned().unwrap();
        tx.send_wait(2, 0).unwrap();
        let waiter = tx.clone();
        let parked = std::thread::spawn(move || waiter.send_wait(3, 0));
        let deadline = Instant::now() + Duration::from_secs(2);
        while rx.waiting_for_space() == 0 {
            assert!(
                Instant::now() < deadline,
                "reader did not park for held credit"
            );
            std::thread::yield_now();
        }
        rx.close_wait_admission();
        assert_eq!(parked.join().unwrap(), Err(TrySendError::Disconnected(3)));
        assert_eq!(rx.stats().queued_items, 1);
        assert_eq!(rx.stats().reserved_bytes, 2 * message_overhead::<u32>());
        drop(held);
        let terminal = tx.reserve_terminal(0).unwrap();
        terminal.publish(4).unwrap();
        assert_eq!(rx.recv(), Ok(2));
        assert_eq!(rx.recv(), Ok(4));
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn frozen_published_count_excludes_receipts_and_concurrent_replies() {
        let (tx, rx) = channel::<u32>(4096).unwrap();
        tx.try_send(1, 0).unwrap();
        tx.try_send(2, 0).unwrap();
        let turn = rx.stats().queued_items;
        let (ready, go) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            for _ in 0..turn {
                go.recv().unwrap();
                tx.try_send(99, 0).unwrap();
            }
        });
        let mut held = Vec::new();
        for _ in 0..turn {
            held.push(rx.recv_owned().unwrap());
            ready.send(()).unwrap();
        }
        worker.join().unwrap();
        let (values, leases): (Vec<_>, Vec<_>) =
            held.into_iter().map(OwnedReceipt::into_parts).unzip();
        assert_eq!(values, [1, 2]);
        assert_eq!(rx.stats().queued_items, 2);
        assert_eq!(rx.stats().reserved_bytes, 4 * message_overhead::<u32>());
        drop(leases);
        assert_eq!(rx.stats().reserved_bytes, 2 * message_overhead::<u32>());
    }

    #[test]
    fn pending_publication_and_abort_share_budget() {
        let charge = message_overhead::<u32>() + 10;
        let (tx, rx) = channel(charge * 2).unwrap();
        let outer = tx.reserve(10).unwrap();
        let inner = tx.clone().reserve(10).unwrap();
        assert!(matches!(tx.reserve(0), Err(ReserveError::Full)));
        assert_eq!(tx.try_send(9, 0), Err(TrySendError::Full(9)));
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(tx.stats().reserved_bytes, charge * 2);
        assert_eq!(tx.stats().queued_items, 0);
        drop(inner);
        outer.publish(42).unwrap();
        assert_eq!(tx.stats().reserved_bytes, charge);
        assert_eq!(tx.stats().queued_items, 1);
        assert_eq!(rx.recv(), Ok(42));
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert_eq!(tx.stats().peak_reserved_bytes, charge * 2);
    }

    #[test]
    fn permits_cancel_on_either_endpoint_close() {
        let (tx, rx) = channel::<u32>(1024).unwrap();
        let first = tx.reserve(10).unwrap();
        let second = tx.reserve(10).unwrap();
        drop(rx);
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert_eq!(first.publish(1), Err(1));
        drop(second);
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert!(matches!(tx.reserve(0), Err(ReserveError::Disconnected)));

        let (tx, rx) = channel::<u32>(1024).unwrap();
        tx.try_send(7, 0).unwrap();
        let first = tx.reserve(10).unwrap();
        let second = tx.reserve(10).unwrap();
        drop(tx);
        assert_eq!(rx.stats().reserved_bytes, message_overhead::<u32>());
        assert_eq!(first.publish(1), Err(1));
        drop(second);
        assert_eq!(rx.recv(), Ok(7));
        assert_eq!(rx.recv(), Err(RecvError));
        assert_eq!(rx.stats().reserved_bytes, 0);
    }

    #[test]
    fn concurrent_pending_flood_is_bounded() {
        for bytes in [0, 257] {
            let charge = message_overhead::<Vec<u8>>() + bytes;
            let (tx, rx) = channel::<Vec<u8>>(31 * charge).unwrap();
            let held = std::thread::scope(|scope| {
                let workers: Vec<_> = (0..8)
                    .map(|_| {
                        let tx = tx.clone();
                        scope.spawn(move || {
                            (0..1000)
                                .filter_map(|_| tx.reserve(bytes).ok())
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .flat_map(|worker| worker.join().unwrap())
                    .collect::<Vec<_>>()
            });
            assert_eq!(held.len(), 31);
            assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
            assert_eq!(tx.stats().reserved_bytes, 31 * charge);
            assert_eq!(tx.stats().peak_reserved_bytes, 31 * charge);
            drop(held);
            assert_eq!(tx.stats().reserved_bytes, 0);
        }
    }

    #[test]
    fn concurrent_flood_bounds_heap_and_empty_messages() {
        for payload in [0, 257] {
            let charge = message_overhead::<Vec<u8>>() + payload;
            let capacity = charge * 31;
            let (tx, rx) = channel(capacity).unwrap();
            std::thread::scope(|scope| {
                for _ in 0..8 {
                    let tx = tx.clone();
                    scope.spawn(move || {
                        for _ in 0..10_000 {
                            let value = vec![0; payload];
                            let _ = tx.try_send(value, payload);
                        }
                    });
                }
            });
            assert_eq!(
                tx.stats(),
                Stats {
                    reserved_bytes: capacity,
                    peak_reserved_bytes: capacity,
                    queued_items: 31
                }
            );
            let mut retained_heap = 0;
            for _ in 0..31 {
                retained_heap += rx.try_recv().unwrap().capacity();
            }
            assert_eq!(retained_heap, payload * 31);
            assert_eq!(tx.stats().reserved_bytes, 0);
            tx.try_send(Vec::new(), 0).unwrap();
            assert!(rx.recv().unwrap().is_empty());
            assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
            assert_eq!(
                tx.try_send(Vec::new(), usize::MAX),
                Err(TrySendError::Full(Vec::new()))
            );
            assert_eq!(tx.stats().reserved_bytes, 0);
        }
    }

    #[test]
    fn disconnect_releases_values_and_budget() {
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = channel::<Counted>(1024).unwrap();
        assert!(tx.try_send(Counted(drops.clone()), 0).is_ok());
        drop(rx);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert!(matches!(
            tx.try_send(Counted(drops.clone()), 0),
            Err(TrySendError::Disconnected(_))
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(tx.stats().queued_items, 0);
    }

    #[test]
    fn timed_owned_receive_keeps_credit_through_timeout_and_endpoint_close() {
        let capacity = message_overhead::<u32>() + 4;
        let (tx, rx) = channel::<u32>(capacity).unwrap();
        assert!(matches!(
            rx.recv_timeout_owned(Duration::ZERO),
            Err(RecvTimeoutError::Timeout)
        ));
        tx.try_send(42, 4).unwrap();
        let (value, credit) = rx.recv_timeout_owned(Duration::ZERO).unwrap().into_parts();
        assert_eq!(value, 42);
        assert_eq!(tx.stats().reserved_bytes, capacity);
        assert!(matches!(tx.reserve(4), Err(ReserveError::Full)));
        assert!(matches!(
            rx.recv_timeout_owned(Duration::ZERO),
            Err(RecvTimeoutError::Timeout)
        ));
        drop(rx);
        assert_eq!(tx.stats().reserved_bytes, capacity);
        drop(credit);
        assert_eq!(tx.stats().reserved_bytes, 0);
    }

    #[test]
    fn blocking_delivery_timeout_and_last_sender_close() {
        assert!(channel::<()>(0).is_err());
        let (tx, rx) = channel::<u32>(1024).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::ZERO),
            Err(RecvTimeoutError::Timeout)
        );
        std::thread::scope(|scope| {
            scope.spawn(move || {
                tx.try_send(42, 0).unwrap();
            });
            assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(42));
            assert_eq!(rx.recv(), Err(RecvError));
            assert_eq!(
                rx.recv_timeout(Duration::ZERO),
                Err(RecvTimeoutError::Disconnected)
            );
        });
    }

    #[test]
    fn held_pressure_is_told_from_a_backlog_of_ordinary_credit() {
        let charge = message_overhead::<u32>() + 16;
        let (tx, _rx) = channel::<u32>(charge * 2).unwrap();
        let held = tx.reserve_revocable(16).unwrap();
        let held_too = tx.reserve_revocable(16).unwrap();
        // Full, but only the holds fill it: the ordinary send would fit without them.
        assert!(matches!(tx.reserve(16), Err(ReserveError::Full)));
        assert!(tx.fits_without_revocable(16));
        drop((held, held_too));
        let _ordinary = tx.reserve(16).unwrap();
        let _more = tx.reserve(16).unwrap();
        // Ordinary credit fills it: no refusal here is the holds' doing.
        assert!(!tx.fits_without_revocable(16));
    }

    #[test]
    fn publishing_held_permits_does_not_exhaust_the_excluding_sends_credit() {
        let charge = message_overhead::<u32>() + 16;
        let (tx, rx) = channel::<u32>(charge * 2).unwrap();
        let first = tx.reserve_revocable(16).unwrap();
        let second = tx.reserve_revocable(16).unwrap();
        tx.try_send_excluding_revocable(1, 16).unwrap();
        // Publishing the permits fills the ordinary credit on top of the queued reply.
        first.publish(2).unwrap();
        second.publish(3).unwrap();
        // Neither class alone is exhausted, so the next reply is still admitted.
        tx.try_send_excluding_revocable(4, 16).unwrap();
        assert_eq!(
            tx.try_send_excluding_revocable(5, 16),
            Err(TrySendError::Full(5))
        );
        // FIFO is untouched, and dequeuing refunds each class.
        assert_eq!(
            [rx.recv(), rx.recv(), rx.recv(), rx.recv()],
            [Ok(1), Ok(2), Ok(3), Ok(4)]
        );
        assert_eq!(tx.stats().reserved_bytes, 0);
        tx.try_send_excluding_revocable(6, 16).unwrap();
        tx.try_send_excluding_revocable(7, 16).unwrap();
    }

    #[test]
    fn revocable_permits_can_be_revoked_and_do_not_block_excluding_sends() {
        let charge = message_overhead::<u32>() + 16;
        let (tx, rx) = channel::<u32>(charge * 2).unwrap();
        let kept = tx.reserve(16).unwrap();
        let revocable = tx.reserve_revocable(16).unwrap();
        assert!(matches!(tx.reserve(0), Err(ReserveError::Full)));
        assert!(matches!(tx.reserve_revocable(0), Err(ReserveError::Full)));
        assert_eq!(tx.try_send(9, 0), Err(TrySendError::Full(9)));

        // Replies sent this way have their own capacity, unaffected by the permits held
        // (either kind) or published later: two fit, a third is refused.
        tx.try_send_excluding_revocable(10, 16).unwrap();
        tx.try_send_excluding_revocable(11, 16).unwrap();
        assert_eq!(
            tx.try_send_excluding_revocable(12, 16),
            Err(TrySendError::Full(12))
        );
        // The overshoot must not underflow reserve().
        assert!(matches!(tx.reserve(0), Err(ReserveError::Full)));
        assert_eq!(rx.recv(), Ok(10));
        assert_eq!(rx.recv(), Ok(11));
        assert_eq!(tx.stats().reserved_bytes, charge * 2);

        // Revoking refunds only revocable credit without disturbing non-revocable permits.
        tx.revoke_pending();
        assert_eq!(tx.stats().reserved_bytes, charge);
        assert_eq!(revocable.publish(99), Err(99));
        assert_eq!(tx.stats().reserved_bytes, charge);

        // A new revocable permit in the next epoch still charges and publishes normally.
        let fresh = tx.reserve_revocable(16).unwrap();
        assert_eq!(tx.stats().reserved_bytes, charge * 2);
        fresh.publish(20).unwrap();
        kept.publish(30).unwrap();
        assert_eq!(rx.recv(), Ok(20));
        assert_eq!(rx.recv(), Ok(30));
        assert_eq!(tx.stats().reserved_bytes, 0);
        assert_eq!(tx.stats().queued_items, 0);
    }
}
