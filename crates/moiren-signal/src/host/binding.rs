use super::slot::{CellRoot, Owner, Pointer};
use crate::{
    Shape, TopicId,
    budget::Charge,
    lifecycle::{StateHandle, TopicLease},
};
use std::{any::TypeId, marker::PhantomData};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostId(pub(crate) u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingKey {
    pub(crate) host: HostId,
    pub(crate) value: u64,
}

pub(crate) struct SlotRoot {
    pub(crate) state: StateHandle,
    pub(crate) cell: CellRoot,
}

pub(crate) struct BindingOwner {
    pub(crate) host: HostId,
    pub(crate) key: BindingKey,
    pub(crate) identity: u64,
    pub(crate) topic: TopicId,
    pub(crate) capacity: usize,
    pub(crate) shape: TypeId,
    pub(crate) pointer: Pointer,
    pub(crate) refresh: fn(Pointer, crate::DemandSnapshot),
    pub(crate) slot: Owner<SlotRoot>,
    pub(crate) _lease: TopicLease,
}

/// Typed identity only. Publication requires an active operation scope.
///
/// Latest value function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestValue, PublisherHandle};
/// fn coerce_latest_value(original: PublisherHandle<LatestValue<fn(&u32)>>)
///     -> PublisherHandle<LatestValue<fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Stream value function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamValue, PublisherHandle};
/// fn coerce_stream_value(original: PublisherHandle<StreamValue<fn(&u32)>>)
///     -> PublisherHandle<StreamValue<fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Latest block header function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestBlock, PublisherHandle};
/// fn coerce_latest_block_header(original: PublisherHandle<LatestBlock<fn(&u32), u32>>)
///     -> PublisherHandle<LatestBlock<fn(&'static u32), u32>>
/// {
///     original
/// }
/// ```
///
/// Latest block element function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestBlock, PublisherHandle};
/// fn coerce_latest_block_element(original: PublisherHandle<LatestBlock<u32, fn(&u32)>>)
///     -> PublisherHandle<LatestBlock<u32, fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Stream block header function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamBlock, PublisherHandle};
/// fn coerce_stream_block_header(original: PublisherHandle<StreamBlock<fn(&u32), u32>>)
///     -> PublisherHandle<StreamBlock<fn(&'static u32), u32>>
/// {
///     original
/// }
/// ```
///
/// Stream block element function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamBlock, PublisherHandle};
/// fn coerce_stream_block_element(original: PublisherHandle<StreamBlock<u32, fn(&u32)>>)
///     -> PublisherHandle<StreamBlock<u32, fn(&'static u32)>>
/// {
///     original
/// }
/// ```
pub struct PublisherHandle<P: Shape> {
    pub(crate) host: HostId,
    pub(crate) key: BindingKey,
    pub(crate) identity: u64,
    pub(crate) topic: TopicId,
    pub(crate) pointer: Pointer,
    // The host casts this token back to its prepared typed cell; keep P
    // invariant so a safe lifetime coercion cannot change that payload type.
    pub(crate) marker: PhantomData<fn(P) -> P>,
}
impl<P: Shape> Copy for PublisherHandle<P> {}
impl<P: Shape> Clone for PublisherHandle<P> {
    fn clone(&self) -> Self {
        *self
    }
}

/// Preparation-side lease. Clone/erase/drop are non-RT operations.
///
/// Latest value function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestValue, PreparedPublisher};
/// fn coerce_latest_value(original: PreparedPublisher<LatestValue<fn(&u32)>>)
///     -> PreparedPublisher<LatestValue<fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Stream value function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamValue, PreparedPublisher};
/// fn coerce_stream_value(original: PreparedPublisher<StreamValue<fn(&u32)>>)
///     -> PreparedPublisher<StreamValue<fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Latest block header function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestBlock, PreparedPublisher};
/// fn coerce_latest_block_header(original: PreparedPublisher<LatestBlock<fn(&u32), u32>>)
///     -> PreparedPublisher<LatestBlock<fn(&'static u32), u32>>
/// {
///     original
/// }
/// ```
///
/// Latest block element function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{LatestBlock, PreparedPublisher};
/// fn coerce_latest_block_element(original: PreparedPublisher<LatestBlock<u32, fn(&u32)>>)
///     -> PreparedPublisher<LatestBlock<u32, fn(&'static u32)>>
/// {
///     original
/// }
/// ```
///
/// Stream block header function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamBlock, PreparedPublisher};
/// fn coerce_stream_block_header(original: PreparedPublisher<StreamBlock<fn(&u32), u32>>)
///     -> PreparedPublisher<StreamBlock<fn(&'static u32), u32>>
/// {
///     original
/// }
/// ```
///
/// Stream block element function-pointer payloads cannot be retyped:
///
/// ```compile_fail
/// use moiren_signal::{StreamBlock, PreparedPublisher};
/// fn coerce_stream_block_element(original: PreparedPublisher<StreamBlock<u32, fn(&u32)>>)
///     -> PreparedPublisher<StreamBlock<u32, fn(&'static u32)>>
/// {
///     original
/// }
/// ```
pub struct PreparedPublisher<P: Shape> {
    pub(crate) binding: ErasedBinding,
    // Prepared authority and its derived handle must have identical P types.
    pub(crate) marker: PhantomData<fn(P) -> P>,
}
impl<P: Shape> PreparedPublisher<P> {
    /// Immutable element capacity from preparation; value topics report zero.
    pub fn capacity(&self) -> usize {
        self.binding.owner.capacity
    }
    pub fn handle(&self) -> PublisherHandle<P> {
        let owner = &self.binding.owner;
        PublisherHandle {
            host: owner.host,
            key: owner.key,
            identity: owner.identity,
            topic: owner.topic,
            pointer: owner.pointer,
            marker: PhantomData,
        }
    }
    pub fn erase(&self) -> ErasedBinding {
        self.binding.clone()
    }
}

#[derive(Clone)]
pub struct ErasedBinding {
    pub(crate) owner: Owner<BindingOwner>,
}

impl ErasedBinding {
    /// Immutable topic, complete typed transport shape and element capacity.
    pub fn schema(&self) -> (TopicId, TypeId, usize) {
        (self.owner.topic, self.owner.shape, self.owner.capacity)
    }
    /// Returns the preparation key without exposing the erased payload pointer.
    pub fn key(&self) -> BindingKey {
        self.owner.key
    }
}

/// Prevalidated per-operation table. Creating or dropping it is non-RT.
pub struct OpSignalBindings {
    pub(crate) table: u64,
    pub(crate) host: HostId,
    pub(crate) key: BindingKey,
    pub(crate) bindings: Box<[ErasedBinding]>,
    pub(crate) _root: Owner<super::HostRoot>,
    pub(crate) _charge: Charge,
}

impl OpSignalBindings {
    /// Build a smaller operation view from this already validated table.
    /// Preparation remains non-RT; the returned view retains the same host and
    /// table identity and rechecks every selected binding against the source.
    pub fn for_operation(&self, bindings: &[ErasedBinding]) -> Result<Self, crate::HostError> {
        if bindings.is_empty()
            || bindings.iter().any(|binding| {
                binding.owner.host != self.host
                    || binding.owner.key != self.key
                    || !self.bindings.iter().any(|candidate| {
                        candidate.owner.identity == binding.owner.identity
                            && candidate.owner.topic == binding.owner.topic
                            && candidate.owner.pointer == binding.owner.pointer
                    })
            })
        {
            return Err(crate::HostError::UnknownBinding);
        }
        let charge = crate::host::table_charge(
            &self._root.budget,
            bindings.len(),
            std::mem::size_of::<Self>(),
        )?;
        Ok(Self {
            table: self.table,
            host: self.host,
            key: self.key,
            bindings: bindings.to_vec().into_boxed_slice(),
            _root: self._root.clone(),
            _charge: charge,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{BlockInit, Bus};

    #[test]
    fn latest_block_cell_header_length_and_sequence_survive_activation() {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus
            .register_latest_block(
                "block-activation",
                BlockInit {
                    header: (0_u64, 0_u64),
                    element: 0_f32,
                    capacity: 4,
                },
            )
            .unwrap();
        let mut reader = bus.subscribe(&topic, Default::default()).unwrap();
        let (control, mut host) = bus.create_host(1).unwrap();
        let original = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        let pointer = original.handle().pointer;
        let update = control.prepare_update(&[original.erase()]).unwrap();
        let old = update.op_bindings(original.erase().key()).unwrap();
        drop(host.activate(update).unwrap());
        host.begin_block();
        host.scope(&old)
            .publish_slice(&original.handle(), (100, 103), &[0.25, 0.5, 0.75])
            .unwrap();
        let replacement = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        let mut update = control.prepare_update(&[replacement.erase()]).unwrap();
        let next = update.op_bindings(replacement.erase().key()).unwrap();
        let retired = host.activation(&mut update).unwrap().commit();
        assert_eq!(
            host.active.as_ref().unwrap().bindings[0]
                .owner
                .pointer
                .address(),
            pointer.address()
        );
        assert_eq!(replacement.handle().pointer.address(), pointer.address());
        bus.pump(crate::PumpBudget {
            topic_checks: 1024,
            deliveries: 128,
            copy_bytes: 65536,
        });
        let block = reader.read_latest().unwrap();
        assert_eq!(*block.header(), (100, 103));
        assert_eq!(block.elements(), &[0.25, 0.5, 0.75]);
        assert_eq!((block.meta().len, block.meta().sequence), (3, 1));
        let generation = block.meta().activation_generation;
        drop(block);
        host.begin_block();
        host.scope(&next)
            .publish_slice(&replacement.handle(), (103, 104), &[0.9])
            .unwrap();
        bus.pump(crate::PumpBudget {
            topic_checks: 1024,
            deliveries: 128,
            copy_bytes: 65536,
        });
        let block = reader.read_latest().unwrap();
        assert_eq!(*block.header(), (103, 104));
        assert_eq!(block.elements(), &[0.9]);
        assert_eq!((block.meta().len, block.meta().sequence), (1, 2));
        assert_eq!(block.meta().activation_generation, generation);
        assert_eq!(
            host.active.as_ref().unwrap().bindings[0]
                .owner
                .pointer
                .address(),
            pointer.address()
        );
        drop(retired);
    }

    #[test]
    fn erased_schema_retains_actual_topic_shape_capacity_and_cell() {
        use std::any::TypeId;
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus
            .register_latest_block(
                "schema",
                BlockInit {
                    header: 0_u64,
                    element: 0_f32,
                    capacity: 2,
                },
            )
            .unwrap();
        let (control, _host) = bus.create_host(2).unwrap();
        let publisher = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        let retained = publisher.erase();
        assert_eq!(
            retained.schema(),
            (topic.id(), TypeId::of::<crate::LatestBlock<u64, f32>>(), 2)
        );
        let replacement = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        assert_eq!(
            retained.owner.pointer.address(),
            replacement.handle().pointer.address()
        );
        assert_ne!(retained.owner.identity, replacement.handle().identity);
        drop(publisher);
        assert_eq!(retained.schema(), replacement.erase().schema());
        let stream = bus
            .register_stream_value("schema-stream", 0_u64, 4)
            .unwrap();
        let original = control
            .prepare(&stream, control.new_binding().unwrap())
            .unwrap();
        let replacement = control
            .prepare(&stream, control.new_binding().unwrap())
            .unwrap();
        assert_eq!(
            original.handle().pointer.address(),
            replacement.handle().pointer.address()
        );
        assert_ne!(original.handle().identity, replacement.handle().identity);
        assert_eq!(
            original.erase().schema(),
            (stream.id(), TypeId::of::<crate::StreamValue<u64>>(), 0)
        );
    }

    #[test]
    fn prepared_capacity_is_immutable_metadata_while_the_host_writes() {
        let mut bus = Bus::new(Default::default()).unwrap();
        let topic = bus
            .register_latest_block(
                "capacity",
                BlockInit {
                    header: 0_u64,
                    element: 0.0_f32,
                    capacity: 2,
                },
            )
            .unwrap();
        let _subscriber = bus.subscribe(&topic, Default::default()).unwrap();
        let (control, mut host) = bus.create_host(2).unwrap();
        let publisher = control
            .prepare(&topic, control.new_binding().unwrap())
            .unwrap();
        assert_eq!(publisher.capacity(), 2);
        let update = control.prepare_update(&[publisher.erase()]).unwrap();
        let op = update.op_bindings(publisher.erase().key()).unwrap();
        drop(host.activate(update).unwrap());
        host.begin_block();
        let mut scope = host.scope(&op);
        let mut write = scope.try_write(&publisher.handle(), 2).unwrap();
        write.elements_mut().fill(3.0);
        assert_eq!(publisher.capacity(), 2);
        write.commit(2).unwrap();
        assert_eq!(publisher.capacity(), 2);
    }
}
