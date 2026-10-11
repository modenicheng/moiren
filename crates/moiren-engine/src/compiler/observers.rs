use super::plan::{Builder, DraftOp};
use crate::{
    processor::{Observer, RtObserver},
    sample::ProcessingSample,
};

pub(super) fn push<S: ProcessingSample>(
    builder: &mut Builder<S>,
    observer: Box<dyn RtObserver<S>>,
    slot: usize,
) -> Result<(), super::CompileError> {
    let processor = builder.processor(Observer(observer))?;
    builder.ops.push(DraftOp {
        processor,
        reads: vec![(0, slot)],
        output: None,
    });
    Ok(())
}
