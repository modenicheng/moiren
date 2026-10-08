use moiren_engine::{buffer::*, sample::ProcessingSample};

fn arena<S: ProcessingSample>() -> BufferArena<S> {
    BufferArena::new(
        &[BufferSlotLayout {
            channels: 2,
            capacity_frames: 8,
        }; 3],
        4096,
    )
    .unwrap()
}
fn read(slot: BufferSlotId, port: u16) -> PortAccess {
    PortAccess::Read { port, slot }
}
fn write(slot: BufferSlotId, port: u16) -> PortAccess {
    PortAccess::Write { port, slot }
}

#[test]
fn checked_layout_budget_and_empty_arena() {
    assert!(matches!(
        BufferArena::<f32>::new(
            &[BufferSlotLayout {
                channels: 0,
                capacity_frames: 8
            }],
            1024
        ),
        Err(BufferError::InvalidLayout)
    ));
    assert!(matches!(
        BufferArena::<f32>::new(
            &[BufferSlotLayout {
                channels: usize::MAX,
                capacity_frames: 8
            }],
            usize::MAX
        ),
        Err(BufferError::SizeOverflow)
    ));
    assert!(matches!(
        BufferArena::<f64>::new(
            &[BufferSlotLayout {
                channels: 2,
                capacity_frames: 8
            }],
            8
        ),
        Err(BufferError::BudgetExceeded)
    ));
    assert_eq!(arena::<f32>().allocated_samples(), 48);
    assert_eq!(
        BufferArena::<f64>::new(&[], 0).unwrap().allocated_samples(),
        0
    );
}

#[test]
fn duplicate_reads_are_legal_but_writable_aliases_are_rejected() {
    let arena = arena::<f32>();
    let a = arena.slot(0).unwrap();
    let b = arena.slot(1).unwrap();
    assert!(
        arena
            .prepare_io(&[read(a, 0), read(a, 1), write(b, 0)])
            .is_ok()
    );
    assert!(matches!(
        arena.prepare_io(&[read(a, 0), write(a, 0)]),
        Err(BufferError::AliasedWrite)
    ));
    assert!(matches!(
        arena.prepare_io(&[write(a, 0), write(a, 1)]),
        Err(BufferError::AliasedWrite)
    ));
    assert!(matches!(
        arena.prepare_io(&[read(a, 0), read(b, 0)]),
        Err(BufferError::DuplicatePort)
    ));
    assert!(matches!(
        arena.prepare_io(&[
            PortAccess::InPlace {
                input: 0,
                output: 0,
                slot: a
            },
            read(a, 1)
        ]),
        Err(BufferError::AliasedWrite)
    ));
    assert!(arena.slot(3).is_none());
}

fn disjoint_roundtrip<S: ProcessingSample>() {
    let mut arena = arena::<S>();
    let a = arena.slot(0).unwrap();
    let b = arena.slot(1).unwrap();
    let c = arena.slot(2).unwrap();
    let source = arena.prepare_io(&[write(a, 0)]).unwrap();
    arena
        .with_io(&source, 0..8, |io| {
            if let ProcessIo::Separate { mut outputs, .. } = io {
                outputs
                    .get_mut(0)
                    .unwrap()
                    .channel_mut(0)
                    .fill(S::from_f64(2.0));
                outputs
                    .get_mut(0)
                    .unwrap()
                    .channel_mut(1)
                    .fill(S::from_f64(3.0));
            }
        })
        .unwrap();
    let process = arena
        .prepare_io(&[read(a, 0), read(a, 1), write(b, 0), write(c, 1)])
        .unwrap();
    arena
        .with_io(&process, 2..5, |io| {
            if let ProcessIo::Separate {
                inputs,
                mut outputs,
            } = io
            {
                let src0 = inputs.get(0).unwrap();
                let src1 = inputs.get(1).unwrap();
                assert_eq!(src0.channel(0).as_ptr(), src1.channel(0).as_ptr());
                let mut outputs = outputs.iter_mut();
                let (_, mut left) = outputs.next().unwrap();
                let (_, mut right) = outputs.next().unwrap();
                // Two output borrows AND repeated input reads remain live together.
                for ch in 0..2 {
                    for i in 0..3 {
                        left.channel_mut(ch)[i] = src0.channel(ch)[i];
                        right.channel_mut(ch)[i] = S::from_f64(src1.channel(ch)[i].to_f64() * 2.0);
                    }
                }
            }
        })
        .unwrap();
    let check = arena.prepare_io(&[read(b, 0), read(c, 1)]).unwrap();
    arena
        .with_io(&check, 0..8, |io| {
            if let ProcessIo::ReadOnly { inputs } = io {
                for (port, block) in inputs.iter() {
                    assert_eq!(block.frames(), 8);
                    assert_eq!(block.channel_count(), 2);
                    for ch in 0..2 {
                        for i in 0..8 {
                            let expected = if (2..5).contains(&i) {
                                (2 + ch) as f64 * (port + 1) as f64
                            } else {
                                0.0
                            };
                            assert_eq!(block.channel(ch)[i].to_f64(), expected);
                        }
                    }
                }
            }
        })
        .unwrap();
}
#[test]
fn disjoint_f32() {
    disjoint_roundtrip::<f32>();
}
#[test]
fn disjoint_f64() {
    disjoint_roundtrip::<f64>();
}

#[test]
fn foreign_proofs_bad_windows_and_move() {
    let mut first = arena::<f32>();
    let mut second = arena::<f32>();
    let access = first
        .prepare_io(&[write(first.slot(0).unwrap(), 0)])
        .unwrap();
    assert!(matches!(
        second.with_io(&access, 0..8, |_| ()),
        Err(BufferError::ForeignAccess)
    ));
    assert!(matches!(
        first.with_io(&access, 0..9, |_| ()),
        Err(BufferError::InvalidFrames)
    ));
    assert!(matches!(
        first.with_io(&access, 8..8, |_| ()),
        Err(BufferError::InvalidFrames)
    ));
    let mut moved = Box::new(first);
    moved.with_io(&access, 0..8, |_| ()).unwrap();
}

#[test]
fn in_place_pairs_can_coexist_with_sidechain_and_aux_output() {
    let mut arena = arena::<f64>();
    let a = arena.slot(0).unwrap();
    let b = arena.slot(1).unwrap();
    let c = arena.slot(2).unwrap();
    let access = arena
        .prepare_io(&[
            PortAccess::InPlace {
                input: 0,
                output: 0,
                slot: a,
            },
            read(b, 1),
            write(c, 2),
        ])
        .unwrap();
    arena
        .with_io(&access, 0..8, |io| {
            if let ProcessIo::InPlace {
                inputs,
                mut outputs,
                mut pairs,
            } = io
            {
                let sidechain = inputs.get(1).unwrap();
                let mut main = pairs.get_mut(0, 0).unwrap();
                let mut aux = outputs.get_mut(2).unwrap();
                main.channel_mut(0).fill(2.0);
                aux.channel_mut(0).copy_from_slice(main.channel(0));
                assert_eq!(sidechain.channel(0), &[0.0; 8]);
            }
        })
        .unwrap();
}

#[test]
fn forget_and_unwind_do_not_release_a_dynamic_borrow_token() {
    let mut arena = arena::<f32>();
    let access = arena
        .prepare_io(&[write(arena.slot(0).unwrap(), 0)])
        .unwrap();
    arena
        .with_io(&access, 0..8, |io| {
            if let ProcessIo::Separate { mut outputs, .. } = io {
                std::mem::forget(outputs.get_mut(0).unwrap());
            }
        })
        .unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        arena
            .with_io(&access, 0..8, |_io| panic!("test unwind"))
            .unwrap();
    }));
    assert!(result.is_err());
    arena
        .with_io(&access, 0..8, |io| {
            if let ProcessIo::Separate { mut outputs, .. } = io {
                outputs.get_mut(0).unwrap().clear();
            }
        })
        .unwrap();
}
