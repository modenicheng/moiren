use super::*;
use moiren_core::graph::NodeId;

// Include metadata, business header and alignment, not only the bin array.
pub(crate) const MAX_SPECTRUM_COPY_BYTES: usize = 256 * (8192 / 2 + 1) * size_of::<f32>() + 128;
pub(crate) const SIGNAL_BYTE_BUDGET: usize = 256 * 1024 * 1024;

impl SignalService {
    pub(crate) fn register_spectrum(
        &mut self,
        node: NodeId,
        settings: SpectrumSettings,
        channels: usize,
        owner_generation: u64,
    ) -> Result<SpectrumBinding, String> {
        if !settings.valid() || !(1..=256).contains(&channels) {
            return Err("Invalid active spectrum shape".into());
        }
        let capacity = channels * settings.bins_per_channel();
        let topic = self
            .bus
            .register_latest_block(
                &format!("node/{node:?}/spectrum"),
                BlockInit {
                    header: SpectrumWindow {
                        timeline_epoch: 0,
                        frame_start: 0,
                        frame_end: 0,
                        sample_rate: 0.,
                        fft_size: settings.fft_size,
                        channels,
                    },
                    element: 0.,
                    capacity,
                },
            )
            .map_err(|e| e.to_string())?;
        let binding = SpectrumBinding {
            topic,
            settings,
            channels,
            owner_generation,
        };
        self.topics.node_spectra.insert(node, binding.clone());
        Ok(binding)
    }

    /// Call only after the bridge drops its prepared publisher/host leases.
    pub(crate) fn unregister_spectrum(&mut self, node: NodeId) -> Result<(), String> {
        if let Some(binding) = self.topics.node_spectra.get(&node) {
            self.bus
                .unregister(&binding.topic)
                .map_err(|e| e.to_string())?;
        }
        self.topics.node_spectra.remove(&node);
        Ok(())
    }

    pub(crate) fn create_spectrum_host(
        &mut self,
    ) -> Result<(moiren_signal::HostControl, moiren_signal::RtHost), String> {
        self.bus.create_host(1).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests;
