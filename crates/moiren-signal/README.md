# moiren-signal

Independent typed Latest/Stream signal storage. Values and block headers/elements
must be `Copy + Send + Sync + 'static`. Registration, subscriptions, reads,
cloning/dropping Topic handles, lifecycle leases and pumping are non-RT.
Publication uses one `RtHost` per owner thread and a typed handle authorized by
a current operation table. Handles are Copy identities and cannot publish alone.

Prepare on non-RT: `Bus::create_host`, `HostControl::new_binding`, `prepare`,
`prepare_update`, then `PreparedHostUpdate::op_bindings`. One key can authorize
multiple topics for one operation. Every update has a separate table identity;
a superseded operation table fails even when its key/handle is reused.
At a block boundary, `RtHost::activation` validates while exclusively borrowing
both host and candidate; dropping the transaction leaves both unchanged.
`commit` moves only preallocated owners and returns `RetiredHostUpdate`.
The owned `activate` convenience API returns the complete candidate in
`ActivateFailure` on error. Candidates prepared against an old basis are stale.

Call `begin_block` to cache active endpoints' demand, then obtain a short
`host.scope(&op)` for cached `demand`, `publish`, `try_write`, or `publish_slice`.
`try_write` returns the existing concrete `BlockWriteGuard` for Latest blocks
and `StreamBlockWriteGuard` for Stream blocks via sealed shape dispatch. Scopes and
block guards borrow the actual exclusive host access and cannot be sent to
another thread. All runtime pointer bindings are typed and checked off-RT.
Stable cells contain the original transport producer and local checked sequence;
updates preserve both and never reset the demand cache. New topic producers and
host slot/table owners are fully initialized and charged before activation.

Keep a cloned `HostControl` cleanup root on non-RT while RT owns the host.
Transfer all returned retired updates, activation failures, and the stopped
host back to non-RT before dropping them; destruction can free heap storage and
final lifecycle leases. The bus and ordinary control-user handles may disappear
while that cleanup root retains the producer resources. Dropping a unique host
on RT violates this ownership contract. After the host is stopped and released
on non-RT, a new host may rebind the same registered topic using the original
producer, queued data, and sequence. Old control clones cannot revive authority.
No concurrent host migration is supported.

Preparation, claiming/reclaiming host ownership, and final lease release share
the topic lifecycle gate. Active and retired tables keep real plan leases, so
activation can change Bound using lock-free stores without allowing close.
Inactive host slots can retain endpoints but grant no publication permission;
once all plan leases are gone, unregister may close such a topic. Its retained
storage stays charged until actual non-RT reclamation.
Preparation can replace a retained Closed slot after checking its lifecycle
gate; replacement peak storage is reserved before releasing the old owner.

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

## Finite custom block example

Run cargo run --locked -p moiren-signal --example custom_block to see one producer thread publish four StreamBlock<FrameHeader, f32> messages alongside a LatestValue<f32>. The producer returns the movable RtHost and operation table after joining; the caller pumps at most eight bounded rounds, reads both independent subscribers, and then drops all returned owners on the non-RT caller thread.

## Verification boundaries

RT publication, scoped block writes, activation, rejection, abort, and generation refresh use preallocated storage and are covered by TLS allocator tests. The allocator is enabled only around those calls; registration, subscription, pumping, thread joins, and final owner cleanup are intentionally outside those measurements. The concurrency test uses finite producer work, channel acknowledgements, and barriers instead of sleeps. Miri is run with its default alias checks; a platform-specific interpreter limitation is recorded in the task evidence rather than hidden with flags.
