//! Validate the owned native format before reading any packet bytes.
use crate::{
    capture::CaptureError, owner::TaskMemory, shared_stream::format::PreparedSharedFormat,
};
use windows::Win32::Media::Audio::WAVEFORMATEX;

pub(super) fn mix_format(
    memory: TaskMemory<WAVEFORMATEX>,
) -> Result<PreparedSharedFormat, CaptureError> {
    PreparedSharedFormat::from_owned(memory, true).map_err(|_| CaptureError::UnsupportedFormat)
}
