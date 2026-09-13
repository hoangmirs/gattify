use std::collections::{BTreeMap, VecDeque};

/// A FIFO with at most one entry in flight.
pub(super) struct ProcedureQueue<T> {
    waiting: VecDeque<T>,
    in_flight: Option<T>,
}

impl<T> Default for ProcedureQueue<T> {
    fn default() -> Self {
        Self {
            waiting: VecDeque::new(),
            in_flight: None,
        }
    }
}

impl<T> ProcedureQueue<T> {
    pub(super) fn push(&mut self, item: T) {
        self.waiting.push_back(item);
    }

    /// Queues `item` ahead of every waiting entry, to run right after the one in flight.
    pub(super) fn push_front(&mut self, item: T) {
        self.waiting.push_front(item);
    }

    /// Moves the next waiting entry in flight when none is, and returns it.
    pub(super) fn start_next(&mut self) -> Option<&T> {
        if self.in_flight.is_some() {
            return None;
        }
        self.in_flight = Some(self.waiting.pop_front()?);
        self.in_flight.as_ref()
    }

    pub(super) fn in_flight(&self) -> Option<&T> {
        self.in_flight.as_ref()
    }

    pub(super) fn in_flight_mut(&mut self) -> Option<&mut T> {
        self.in_flight.as_mut()
    }

    /// Ends the entry in flight when `matches` accepts it.
    pub(super) fn finish(&mut self, matches: impl FnOnce(&T) -> bool) -> Option<T> {
        if self.in_flight.as_ref().is_some_and(matches) {
            self.in_flight.take()
        } else {
            None
        }
    }

    pub(super) fn remove_waiting(&mut self, mut matches: impl FnMut(&T) -> bool) -> Vec<T> {
        let mut removed = Vec::new();
        let mut kept = VecDeque::with_capacity(self.waiting.len());
        for item in self.waiting.drain(..) {
            if matches(&item) {
                removed.push(item);
            } else {
                kept.push_back(item);
            }
        }
        self.waiting = kept;
        removed
    }

    /// Removes every entry, the one in flight first.
    pub(super) fn drain(&mut self) -> Vec<T> {
        self.in_flight
            .take()
            .into_iter()
            .chain(self.waiting.drain(..))
            .collect()
    }
}

enum Slot<T> {
    Pending,
    Ready(T),
    Skipped,
}

/// Hands out entries in the order of their sequence numbers, while they
/// become ready in any order. A number that has not arrived yet holds back
/// every later one.
pub(super) struct InOrder<T> {
    next: u64,
    slots: BTreeMap<u64, Slot<T>>,
}

impl<T> Default for InOrder<T> {
    fn default() -> Self {
        Self {
            next: 0,
            slots: BTreeMap::new(),
        }
    }
}

impl<T> InOrder<T> {
    /// Notes that `sequence` arrived and becomes ready later.
    pub(super) fn expect(&mut self, sequence: u64) {
        self.slots.insert(sequence, Slot::Pending);
    }

    /// Fills `sequence` with its entry, or skips it with `None`.
    pub(super) fn fill(&mut self, sequence: u64, item: Option<T>) {
        self.slots
            .insert(sequence, item.map_or(Slot::Skipped, Slot::Ready));
    }

    /// The next entry in sequence, when it is ready.
    pub(super) fn pop(&mut self) -> Option<T> {
        loop {
            match self.slots.get(&self.next) {
                Some(Slot::Pending) | None => return None,
                Some(Slot::Skipped | Slot::Ready(_)) => {}
            }
            let slot = self.slots.remove(&self.next);
            self.next += 1;
            if let Some(Slot::Ready(item)) = slot {
                return Some(item);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_entry_runs_at_a_time_in_order() {
        let mut queue = ProcedureQueue::default();
        queue.push(1);
        queue.push(2);
        assert_eq!(queue.start_next(), Some(&1));
        assert_eq!(queue.start_next(), None);
        assert_eq!(queue.in_flight(), Some(&1));
        assert_eq!(queue.finish(|item| *item == 2), None);
        assert_eq!(queue.finish(|item| *item == 1), Some(1));
        assert_eq!(queue.start_next(), Some(&2));
    }

    #[test]
    fn a_rollback_runs_right_after_the_entry_in_flight() {
        let mut queue = ProcedureQueue::default();
        queue.push("read");
        queue.push("write");
        queue.start_next();
        queue.push_front("rollback");
        if let Some(item) = queue.in_flight_mut() {
            *item = "subscribe";
        }
        assert_eq!(queue.finish(|_| true), Some("subscribe"));
        assert_eq!(queue.start_next(), Some(&"rollback"));
    }

    #[test]
    fn waiting_entries_leave_without_touching_the_one_in_flight() {
        let mut queue = ProcedureQueue::default();
        for item in 1..=4 {
            queue.push(item);
        }
        queue.start_next();
        assert_eq!(queue.remove_waiting(|item| item % 2 == 0), vec![2, 4]);
        assert_eq!(queue.drain(), vec![1, 3]);
        assert_eq!(queue.start_next(), None);
    }

    #[test]
    fn entries_leave_in_sequence_whatever_order_they_become_ready_in() {
        let mut writes = InOrder::default();
        writes.expect(0);
        writes.fill(1, Some("second"));
        assert_eq!(writes.pop(), None);
        writes.fill(0, Some("first"));
        assert_eq!(writes.pop(), Some("first"));
        assert_eq!(writes.pop(), Some("second"));
        assert_eq!(writes.pop(), None);
    }

    #[test]
    fn a_number_that_has_not_arrived_holds_back_later_ones() {
        let mut writes = InOrder::default();
        writes.fill(1, Some("second"));
        assert_eq!(writes.pop(), None);
        writes.fill(0, None);
        assert_eq!(writes.pop(), Some("second"));
    }
}
