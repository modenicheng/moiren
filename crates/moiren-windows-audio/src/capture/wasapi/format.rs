//! Validate the owned native format before reading any packet bytes.
use super::api;
use crate::{
    capture::CaptureError, owner::TaskMemory, shared_stream::format::PreparedSharedFormat,
};
use windows::Win32::Media::Audio::IAudioClient;

pub(super) fn mix_format(client: &IAudioClient) -> Result<PreparedSharedFormat, CaptureError> {
    let memory = TaskMemory(api("GetMixFormat(capture)", unsafe {
        client.GetMixFormat()
    })?);
    PreparedSharedFormat::from_owned(memory, true).map_err(|_| CaptureError::UnsupportedFormat)
}
