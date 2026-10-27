//! Driver buffer-size rules are separate from the application's menu choices.

use moiren_engine::host::BackendError;

#[derive(Debug, Clone, Copy)]
pub(super) struct BufferSizes {
    pub min: i32,
    pub max: i32,
    pub preferred: i32,
    pub granularity: i32,
}

impl BufferSizes {
    /// Zero selects the driver's preferred size. The default may be a native
    /// non-power-of-two size; explicit menu choices must remain exact powers.
    pub fn resolve(self, requested: u32) -> Result<u32, BackendError> {
        let frames = if requested == 0 {
            u32::try_from(self.preferred).unwrap_or(0)
        } else {
            requested
        };
        if !(64..=4096).contains(&frames) || (requested != 0 && !frames.is_power_of_two()) {
            return Err(BackendError::Unsupported(format!(
                "ASIO {} buffer {frames} frames is outside the supported {}",
                if requested == 0 {
                    "driver-preferred"
                } else {
                    "requested"
                },
                if requested == 0 {
                    "64..=4096 range"
                } else {
                    "power-of-two menu"
                },
            )));
        }
        self.validate(frames)?;
        Ok(frames)
    }

    pub fn validate(self, frames: u32) -> Result<(), BackendError> {
        let requested = i32::try_from(frames).unwrap_or(-1);
        let in_range =
            self.min > 0 && self.max >= self.min && (self.min..=self.max).contains(&requested);
        let supported = in_range
            && match self.granularity {
                // Steinberg specifies minSize, minSize*2, minSize*4, ...
                -1 => {
                    frames.is_multiple_of(self.min as u32)
                        && (frames / self.min as u32).is_power_of_two()
                }
                // Some drivers (FL Studio ASIO) report zero with a variable
                // range. Only the explicitly reported preferred size is safe
                // to admit without inventing a step or substituting a request.
                0 => requested == self.preferred,
                step if step > 0 => (requested - self.min) % step == 0,
                _ => false,
            };
        if supported {
            Ok(())
        } else {
            Err(BackendError::Unsupported(format!(
                "ASIO buffer {frames} frames is unsupported: min={}, max={}, preferred={}, \
                 granularity={}; select an exact driver-supported size",
                self.min, self.max, self.preferred, self.granularity
            )))
        }
    }
}

pub(super) fn validate_options(
    sample_rate: u32,
    buffer_frames: u32,
    input: bool,
    output: bool,
) -> Result<(), BackendError> {
    if !input && !output {
        return Err(BackendError::Unsupported(
            "ASIO needs at least one enabled input or output".into(),
        ));
    }
    if !matches!(sample_rate, 44100 | 48000 | 88200 | 96000) {
        return Err(BackendError::Unsupported(format!(
            "ASIO sample rate {sample_rate} Hz is outside 44100/48000/88200/96000"
        )));
    }
    if buffer_frames != 0
        && (!(64..=4096).contains(&buffer_frames) || !buffer_frames.is_power_of_two())
    {
        return Err(BackendError::Unsupported(format!(
            "ASIO buffer request {buffer_frames} must be 64/128/256/512/1024/2048/4096 frames"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_power_of_two_requests_never_fall_back_to_preferred_size() {
        let sizes = BufferSizes {
            min: 128,
            max: 2048,
            preferred: 512,
            granularity: -1,
        };
        assert!(sizes.validate(128).is_ok());
        assert!(sizes.validate(2048).is_ok());
        assert!(sizes.validate(64).is_err());
        assert!(sizes.validate(192).is_err());
        assert!(sizes.validate(4096).is_err());
    }

    #[test]
    fn stepped_buffers_are_based_at_minimum_and_fixed_buffers_are_exact() {
        let sizes = BufferSizes {
            min: 96,
            max: 1024,
            preferred: 128,
            granularity: 32,
        };
        assert!(sizes.validate(128).is_ok());
        assert!(sizes.validate(144).is_err());
        let fixed = BufferSizes {
            min: 256,
            max: 256,
            preferred: 256,
            granularity: 0,
        };
        assert!(fixed.validate(256).is_ok());
        assert!(fixed.validate(128).is_err());
        assert!(BufferSizes { max: 512, ..fixed }.validate(512).is_err());
        assert!(BufferSizes { min: -1, ..fixed }.validate(256).is_err());
    }

    #[test]
    fn validates_the_entire_requested_menu_and_rejects_inactive_streams() {
        for rate in [44100, 48000, 88200, 96000] {
            for frames in [64, 128, 256, 512, 1024, 2048, 4096] {
                assert!(validate_options(rate, frames, true, true).is_ok());
            }
        }
        assert!(validate_options(48000, 64, false, false).is_err());
        assert!(validate_options(192000, 64, true, true).is_err());
        assert!(validate_options(48000, 65, true, true).is_err());
        assert!(validate_options(48000, 0, true, true).is_ok());
    }

    #[test]
    fn default_uses_actual_preferred_including_native_non_power_sizes() {
        let sizes = BufferSizes {
            min: 64,
            max: 4096,
            preferred: 96,
            granularity: 32,
        };
        assert_eq!(sizes.resolve(0).unwrap(), 96);
        assert_eq!(sizes.resolve(128).unwrap(), 128);
        assert!(sizes.resolve(96).is_err());
        assert!(
            BufferSizes {
                preferred: 0,
                ..sizes
            }
            .resolve(0)
            .is_err()
        );
        assert!(
            BufferSizes {
                preferred: 8192,
                ..sizes
            }
            .resolve(0)
            .is_err()
        );
        assert!(
            BufferSizes {
                preferred: 80,
                ..sizes
            }
            .resolve(0)
            .is_err()
        );
        let doubled = BufferSizes {
            min: 96,
            max: 1536,
            preferred: 192,
            granularity: -1,
        };
        assert_eq!(doubled.resolve(0).unwrap(), 192);
        assert!(doubled.resolve(128).is_err());
        assert!(doubled.resolve(256).is_err());
    }

    #[test]
    fn zero_granularity_range_admits_only_driver_reported_preferred_size() {
        // Actual FL Studio ASIO metadata observed on 2026-10-18. Zero does not
        // describe the variable range's stepping; do not invent valid sizes.
        let sizes = BufferSizes {
            min: 256,
            max: 4096,
            preferred: 512,
            granularity: 0,
        };
        assert_eq!(sizes.resolve(0).unwrap(), 512);
        assert_eq!(sizes.resolve(512).unwrap(), 512);
        for frames in [256, 1024, 2048, 4096] {
            assert!(sizes.resolve(frames).is_err());
        }
    }
}
