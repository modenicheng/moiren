use super::*;
use moiren_windows_audio::vad::{Capabilities, PdoState};
use std::collections::BTreeMap;

#[derive(Default)]
struct Fake {
    next: u64,
    live: BTreeMap<u64, EndpointSnapshot>,
    creates: usize,
    removes: usize,
    max_live: usize,
    remove_breaks_sibling: bool,
    reuse_token: bool,
    sibling_identity_change: bool,
    sibling_gate_change: bool,
    removal_stays_pending: bool,
    creation_not_ready: bool,
}

fn snapshot(spec: Spec, token: u64, state: PdoState) -> EndpointSnapshot {
    snapshot_at_gate_epoch(spec, token, state, 1)
}

fn snapshot_at_gate_epoch(spec: Spec, token: u64, state: PdoState, gate: u64) -> EndpointSnapshot {
    fn put32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    let mut caps = [0; 64];
    for (offset, value) in [
        (0, 64),
        (4, 2),
        (8, 31),
        (12, 64),
        (16, 1),
        (20, 48_000),
        (24, 3),
        (28, 6),
        (32, 6),
        (36, 4),
        (48, 4800),
    ] {
        put32(&mut caps, offset, value);
    }
    put64(&mut caps, 40, 7);
    let mut bytes = [0; 128];
    for (offset, value) in [
        (0, 128),
        (4, 2),
        (16, state as u32),
        (20, spec.intent as u32),
        (24, spec.channels as u32),
        (32, 1),
        (36, u32::from(state == PdoState::Present)),
    ] {
        put32(&mut bytes, offset, value);
    }
    for (offset, value) in [(8, token), (40, 7), (48, token), (64, gate)] {
        put64(&mut bytes, offset, value);
    }
    bytes[72..88].copy_from_slice(&spec.id.to_le_bytes());
    EndpointSnapshot::decode(&bytes, Capabilities::decode(&caps).unwrap()).unwrap()
}

impl Client for Fake {
    fn create(&mut self, spec: Spec) -> Result<EndpointSnapshot> {
        self.next += 1;
        self.creates += 1;
        let token = if self.reuse_token && self.next > 4 {
            1
        } else {
            self.next
        };
        let value = snapshot(
            spec,
            token,
            if self.creation_not_ready {
                PdoState::Creating
            } else {
                PdoState::Present
            },
        );
        self.live.insert(token, value);
        self.max_live = self.max_live.max(self.live.len());
        Ok(value)
    }
    fn query(&mut self, previous: EndpointSnapshot) -> Result<EndpointSnapshot> {
        let current = self
            .live
            .get(&previous.pdo_token().get())
            .copied()
            .ok_or("sibling disappeared")?;
        if (self.sibling_identity_change || self.sibling_gate_change) && self.removes > 0 {
            return Ok(snapshot_at_gate_epoch(
                Spec {
                    id: current.stable_id(),
                    intent: current.intent(),
                    channels: current.channels(),
                },
                current.pdo_token().get() + u64::from(self.sibling_identity_change) * 100,
                PdoState::Present,
                current.gate_epoch() + u64::from(self.sibling_gate_change),
            ));
        }
        Ok(current)
    }
    fn remove(&mut self, previous: EndpointSnapshot) -> Result<EndpointSnapshot> {
        self.live
            .remove(&previous.pdo_token().get())
            .ok_or("remove missing endpoint")?;
        self.removes += 1;
        if self.remove_breaks_sibling {
            self.live.pop_first();
        }
        Ok(snapshot(
            Spec {
                id: previous.stable_id(),
                intent: previous.intent(),
                channels: previous.channels(),
            },
            previous.pdo_token().get(),
            if self.removal_stays_pending {
                PdoState::Removing
            } else {
                PdoState::Removed
            },
        ))
    }
}

#[test]
fn holds_four_combinations_and_recreates_each_before_retiring_all() {
    let mut client = Fake::default();
    exercise_with(&mut client, 2).unwrap();
    assert_eq!(client.max_live, 4);
    assert_eq!(client.creates, 16);
    assert_eq!(client.removes, 16);
    assert!(client.live.is_empty());
}

#[test]
fn missing_sibling_aborts_before_replacing_the_removed_endpoint() {
    let mut client = Fake {
        remove_breaks_sibling: true,
        ..Default::default()
    };
    assert!(exercise_with(&mut client, 1).is_err());
    assert_eq!(client.creates, 4);
    assert_eq!(client.removes, 1);
}

#[test]
fn replacement_must_not_reuse_a_runtime_token() {
    let mut client = Fake {
        reuse_token: true,
        ..Default::default()
    };
    assert!(exercise_with(&mut client, 1).is_err());
}

#[test]
fn sibling_identity_change_is_not_mistaken_for_independent_survival() {
    let mut client = Fake {
        sibling_identity_change: true,
        ..Default::default()
    };
    assert!(exercise_with(&mut client, 1).is_err());
    assert_eq!(client.creates, 4);
    assert_eq!(client.removes, 1);
}

#[test]
fn sibling_gate_epoch_change_aborts_recreation() {
    let mut client = Fake {
        sibling_gate_change: true,
        ..Default::default()
    };
    assert!(exercise_with(&mut client, 1).is_err());
    assert_eq!(client.creates, 4);
    assert_eq!(client.removes, 1);
}

#[test]
fn pending_removal_cannot_authorize_recreation() {
    let mut client = Fake {
        removal_stays_pending: true,
        ..Default::default()
    };
    assert!(exercise_with(&mut client, 1).is_err());
    assert_eq!(client.creates, 4);
    assert_eq!(client.removes, 1);
}

#[test]
fn creation_must_be_audio_ready_before_becoming_a_live_sibling() {
    let mut client = Fake {
        creation_not_ready: true,
        ..Default::default()
    };
    assert!(exercise_with(&mut client, 1).is_err());
    assert_eq!(client.creates, 1);
    assert_eq!(client.removes, 0);
}
