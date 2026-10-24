//! Pure fixed-slot ownership rules shared by the transport and host tests.

#[cfg(any(windows, test))]
use super::REQUEST_SLOTS;
#[cfg(any(windows, test))]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(any(windows, test))]
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// Opaque completion ticket. Reusing a physical slot cannot revive an old ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmTicket {
    pub(super) slot: usize,
    serial: u64,
    owner: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(any(windows, test))]
pub(super) enum SlotError {
    Full,
    Stale,
    Exhausted,
}

#[cfg(any(windows, test))]
pub(super) struct SlotBook {
    slots: [Option<u64>; REQUEST_SLOTS],
    next_serial: u64,
    next_slot: usize,
    owner: u64,
}

#[cfg(any(windows, test))]
impl SlotBook {
    pub(super) fn new() -> Result<Self, SlotError> {
        let owner = NEXT_OWNER
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| SlotError::Exhausted)?;
        Ok(Self {
            slots: [None; REQUEST_SLOTS],
            next_serial: 0,
            next_slot: 0,
            owner,
        })
    }
    pub(super) fn reserve(&mut self) -> Result<PcmTicket, SlotError> {
        let slot = (0..REQUEST_SLOTS)
            .map(|offset| (self.next_slot + offset) % REQUEST_SLOTS)
            .find(|&index| self.slots[index].is_none())
            .ok_or(SlotError::Full)?;
        let serial = self
            .next_serial
            .checked_add(1)
            .ok_or(SlotError::Exhausted)?;
        self.next_serial = serial;
        self.slots[slot] = Some(serial);
        self.next_slot = (slot + 1) % REQUEST_SLOTS;
        Ok(PcmTicket {
            slot,
            serial,
            owner: self.owner,
        })
    }
    pub(super) fn check(&self, ticket: PcmTicket) -> Result<(), SlotError> {
        if ticket.owner != self.owner
            || self.slots.get(ticket.slot).copied().flatten() != Some(ticket.serial)
        {
            return Err(SlotError::Stale);
        }
        Ok(())
    }
    pub(super) fn finish(&mut self, ticket: PcmTicket) -> Result<(), SlotError> {
        self.check(ticket)?;
        self.slots[ticket.slot] = None;
        Ok(())
    }
    pub(super) fn pending(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }
    #[cfg(windows)]
    pub(super) fn retire_slot(&mut self, index: usize) {
        self.slots[index] = None;
    }
    pub(super) fn rebind(&mut self) -> Result<(), SlotError> {
        if self.pending() != 0 {
            return Err(SlotError::Full);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exactly_four_pending_and_only_terminal_completion_releases_a_slot() {
        let mut book = SlotBook::new().unwrap();
        let tickets = std::array::from_fn::<_, REQUEST_SLOTS, _>(|_| book.reserve().unwrap());
        assert_eq!(book.reserve(), Err(SlotError::Full));
        assert_eq!(book.rebind(), Err(SlotError::Full));
        // Cancellation initiation alone does not call finish.
        assert_eq!(book.pending(), 4);
        book.finish(tickets[1]).unwrap();
        let replacement = book.reserve().unwrap();
        assert_eq!(replacement.slot, tickets[1].slot);
        assert_ne!(replacement, tickets[1]);
        assert_eq!(book.finish(tickets[1]), Err(SlotError::Stale));
        for ticket in [tickets[0], replacement, tickets[2], tickets[3]] {
            book.finish(ticket).unwrap();
        }
        assert_eq!(book.pending(), 0);
    }
    #[test]
    fn session_sequences_and_ticket_counters_do_not_wrap() {
        let mut book = SlotBook::new().unwrap();
        let sequences = AtomicU64::new(0);
        let next = || {
            sequences
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    value.checked_add(1)
                })
                .map(|previous| previous + 1)
        };
        assert_eq!(next(), Ok(1));
        book.rebind().unwrap();
        assert_eq!(next(), Ok(2));
        let mut replacement = SlotBook::new().unwrap();
        replacement.rebind().unwrap();
        assert_eq!(next(), Ok(3));
        sequences.store(u64::MAX, Ordering::Relaxed);
        assert_eq!(next(), Err(u64::MAX));
        book.next_serial = u64::MAX;
        assert_eq!(book.reserve(), Err(SlotError::Exhausted));
        assert_eq!(book.pending(), 0);
    }
    #[test]
    fn tickets_cannot_cross_owners() {
        let mut a = SlotBook::new().unwrap();
        let mut b = SlotBook::new().unwrap();
        let ticket = a.reserve().unwrap();
        let other = b.reserve().unwrap();
        assert_eq!(a.check(other), Err(SlotError::Stale));
        assert_eq!(b.check(ticket), Err(SlotError::Stale));
    }
    #[test]
    fn single_pending_request_rotates_all_preallocated_slots() {
        let mut book = SlotBook::new().unwrap();
        for expected in [0, 1, 2, 3, 0, 1, 2, 3] {
            let ticket = book.reserve().unwrap();
            assert_eq!(ticket.slot, expected);
            assert_eq!(book.pending(), 1);
            book.finish(ticket).unwrap();
        }
    }
}
