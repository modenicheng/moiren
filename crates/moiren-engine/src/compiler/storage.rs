//! Assign physical storage only after lowering has made every read explicit.
use super::plan::DraftOp;
use crate::buffer::BufferSlotLayout;

fn resolve(aliases: &mut [usize], mut slot: usize) -> usize {
    while aliases[slot] != slot {
        aliases[slot] = aliases[aliases[slot]];
        slot = aliases[slot];
    }
    slot
}

pub(super) fn optimize(ops: &mut [DraftOp], layouts: &mut Vec<BufferSlotLayout>) {
    let mut aliases: Vec<_> = (0..layouts.len()).collect();
    let mut last_use = vec![0; layouts.len()];
    let mut first_def = vec![usize::MAX; layouts.len()];
    for (index, op) in ops.iter().enumerate() {
        for &(_, slot) in &op.reads {
            last_use[slot] = last_use[slot].max(index);
        }
        for &(_, slot) in &op.outputs {
            first_def[slot] = first_def[slot].min(index);
            last_use[slot] = last_use[slot].max(index);
        }
    }
    for (index, op) in ops.iter_mut().enumerate() {
        if !op.in_place_candidate || op.reads.len() != 1 || op.outputs.len() != 1 {
            continue;
        }
        let input = resolve(&mut aliases, op.reads[0].1);
        let output = resolve(&mut aliases, op.outputs[0].1);
        // Slots without a writer are persistent silence across all blocks and
        // event segments. Never turn one into writable or recycled storage.
        if input != output
            && first_def[input] != usize::MAX
            && last_use[input] == index
            && layouts[input].channels == layouts[output].channels
        {
            aliases[output] = input;
            last_use[input] = last_use[input].max(last_use[output]);
            op.in_place = true;
        }
    }

    let mut roots: Vec<_> = (0..layouts.len())
        .filter(|&slot| resolve(&mut aliases, slot) == slot)
        .collect();
    roots.sort_by_key(|&slot| first_def[slot]);
    let mut physical: Vec<(BufferSlotLayout, usize)> = Vec::new();
    let mut root_to_physical = vec![0; layouts.len()];
    for root in roots {
        let start = first_def[root];
        let end = if start == usize::MAX {
            usize::MAX
        } else {
            last_use[root]
        };
        // Strict inequality preserves separate read/write storage even at the
        // last consumer's own op. Only the in-place proof above may alias it.
        let reusable = (start != usize::MAX)
            .then(|| {
                physical.iter().position(|(layout, last)| {
                    *last < start
                        && layout.channels == layouts[root].channels
                        && layout.capacity_frames == layouts[root].capacity_frames
                })
            })
            .flatten();
        let index = if let Some(index) = reusable {
            physical[index].1 = end;
            index
        } else {
            let index = physical.len();
            physical.push((layouts[root], end));
            index
        };
        root_to_physical[root] = index;
    }
    // Logical aliases and physical IDs are separate index spaces. Resolve all
    // logical roots before rewriting any operation's slot IDs.
    let physical_slots: Vec<_> = (0..layouts.len())
        .map(|slot| root_to_physical[resolve(&mut aliases, slot)])
        .collect();
    for op in ops {
        for (_, slot) in op.reads.iter_mut().chain(&mut op.outputs) {
            *slot = physical_slots[*slot];
        }
    }
    *layouts = physical.into_iter().map(|(layout, _)| layout).collect();
}
