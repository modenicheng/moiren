# moiren-signal

Independent typed Latest/Stream signal storage. Values and block headers/elements
must be `Copy + Send + Sync + 'static`. Registration, subscriptions, reads,
cloning/dropping Topic handles, lifecycle leases and pumping are non-RT.
Publication endpoints remain crate-private until scoped RT host authority is added.

`Bus::pump(PumpBudget)` visits fixed directory slots round-robin. Every slot visit,
including an empty slot, uses one `topic_checks`. Each source-to-staging copy,
recipient inspection/delivery, stale ingress discard, and retirement step uses
one `deliveries`. A copy charges the complete transport layout: metadata plus
value, or metadata/header plus the used block elements and layout padding.
Each subscriber copy is charged independently. Stream staging is prepared at
registration and returns ingress owners immediately after the source copy.

Fanout retains its source and recipient bitmap across calls. A late subscriber
does not replay older Stream messages. Latest seeding counts as delivery and
cannot regress when an older fanout resumes. Full Stream subscriptions drop new
deliveries locally; ingress Full attempts and subscriber overflow are distinct
saturating statistics accessible through `Topic::stats` and `Subscriber::stats`.

If an atomic copy does not fit, `required_copy_bytes` reports its complete size;
no partial message is consumed or copied. Increase both delivery and byte budgets
to make progress. Other topics continue to receive their round-robin visits.
`has_pending` includes known work and conservatively includes unexamined slots
when the check budget prevents a complete quiet directory pass. No worker or
wakeup is created; the caller chooses when to pump again.

`Bus::unregister(&topic)` returns `Busy` without changing anything while a
publisher binding or lifecycle lease remains. Success closes the old state and
removes its name. New same-name registration has a different identity. Status
distinguishes `Closed`, `NoPublisher`, and an empty read; previously held guards
remain valid. Removing a publisher alone keeps old snapshots and queued messages.

Budgeting models actual allocation layouts, including staging, receiver loans,
handoff metadata and shared reservation allocations. Retired and closed resources
remain charged until their owners actually release every allocation on non-RT.
Pump retirement frees at most one queued payload owner per charged step and
alternates with live Stream work; the final empty rings have constant cleanup.
Explicit unregister, preparation and final non-RT Drop are not pump-budgeted.
