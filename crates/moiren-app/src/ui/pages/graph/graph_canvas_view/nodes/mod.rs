mod card;
mod chips;
mod curve_thumb;
mod dial;
mod faults;
mod geometry;
mod meters;
mod previews;
mod recording;
mod shared;
mod spectrum;

pub(in crate::ui) use card::paint_node;
pub(in crate::ui) use faults::{paint_fault_banner, paint_fault_tooltip};
pub(in crate::ui) use meters::paint_meter_bars;
pub(in crate::ui) use recording::paint_recording_duration;
pub(in crate::ui) use shared::{CanvasShapesHandle, Paint};
pub(in crate::ui) use spectrum::paint_spectrum_body;
// The file-row and transport-look decisions, re-exported for the canvas
// layout tests (see the parent module's cfg(test) re-export).
#[cfg(test)]
pub(in crate::ui) use recording::{strip_file_label, transport_look};
