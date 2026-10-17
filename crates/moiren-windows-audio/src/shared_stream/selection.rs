//! Pure format policy shared by stream preparation and catalog admission.
use super::SharedFormatInfo;

impl SharedFormatInfo {
    pub(super) fn valid_layout(self, align: u16, bytes_per_second: u32) -> bool {
        let valid_container = if self.floating_point {
            [32, 64].contains(&self.container_bits)
        } else {
            [8, 16, 24, 32].contains(&self.container_bits)
        };
        self.sample_rate != 0
            && (1..=32).contains(&self.channels)
            && valid_container
            && self.valid_bits.is_none_or(|bits| {
                bits != 0
                    && bits <= self.container_bits
                    && (!self.floating_point || bits == self.container_bits)
            })
            && self
                .channel_mask
                .is_none_or(|mask| mask == 0 || mask.count_ones() == u32::from(self.channels))
            && self.channels.checked_mul(self.container_bits / 8) == Some(align)
            && self.sample_rate.checked_mul(u32::from(align)) == Some(bytes_per_second)
    }

    pub(super) fn select_client(self, capture: bool) -> Option<Self> {
        let standard_mask = match (capture, self.channels) {
            (true, 1) => 4, // SPEAKER_FRONT_CENTER
            (_, 2) => 3,    // SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT
            _ => return None,
        };
        // A conversion flag is not authorization for a channel mapper. Unknown
        // masks retain conventional mono/stereo ordering; explicit other masks
        // must not be silently reinterpreted as front channels.
        if self
            .channel_mask
            .is_some_and(|mask| mask != 0 && mask != standard_mask)
        {
            return None;
        }
        let native = self.floating_point
            && self.container_bits == 32
            && self.valid_bits.is_none_or(|bits| bits == 32)
            && if capture {
                [44_100, 48_000].contains(&self.sample_rate)
            } else {
                self.sample_rate == 48_000
            };
        Some(if native {
            self
        } else {
            Self {
                sample_rate: 48_000,
                channels: self.channels,
                container_bits: 32,
                valid_bits: None,
                channel_mask: None,
                floating_point: true,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mix(channels: u16, mask: Option<u32>) -> SharedFormatInfo {
        SharedFormatInfo {
            sample_rate: 48_000,
            channels,
            container_bits: 32,
            valid_bits: None,
            channel_mask: mask,
            floating_point: true,
        }
    }

    #[test]
    fn rejects_surround_capture_and_nonstereo_render_for_pcm_and_float() {
        for capture in [false, true] {
            for channels in [1, 2, 4, 6, 8, 32] {
                for floating_point in [false, true] {
                    for mask in [None, Some(0), Some(u32::MAX >> (32 - channels))] {
                        let format = SharedFormatInfo {
                            floating_point,
                            ..mix(channels, mask)
                        };
                        if channels > 2 || (!capture && channels == 1) {
                            assert_eq!(format.select_client(capture), None);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn accepts_only_conventional_mono_and_front_stereo_masks() {
        for capture in [false, true] {
            for channels in [1, 2] {
                for floating_point in [false, true] {
                    for mask in [None, Some(0), Some(3), Some(4), Some(1), Some(0xc)] {
                        let format = SharedFormatInfo {
                            floating_point,
                            ..mix(channels, mask)
                        };
                        let expected = (channels == 2 || capture)
                            && mask.is_none_or(|mask| {
                                mask == 0 || mask == if channels == 1 { 4 } else { 3 }
                            });
                        assert_eq!(format.select_client(capture).is_some(), expected);
                    }
                }
            }
        }
    }

    #[test]
    fn native_and_converted_clients_preserve_mono_or_stereo_layout() {
        for (capture, channels) in [(true, 1), (true, 2), (false, 2)] {
            for rate in [22_050, 44_100, 48_000, 96_000, 192_000] {
                for (floating_point, bits) in [(false, 8), (false, 24), (true, 32), (true, 64)] {
                    let format = SharedFormatInfo {
                        sample_rate: rate,
                        container_bits: bits,
                        floating_point,
                        ..mix(channels, None)
                    };
                    let align = channels * (bits / 8);
                    assert!(format.valid_layout(align, rate * u32::from(align)));
                    let client = format.select_client(capture).unwrap();
                    assert_eq!(client.channels, channels);
                    assert!(client.floating_point);
                    assert_eq!(client.container_bits, 32);
                    let native = floating_point
                        && bits == 32
                        && (rate == 48_000 || (capture && rate == 44_100));
                    assert_eq!(client == format, native);
                    assert_eq!(client.sample_rate, if native { rate } else { 48_000 });
                }
            }
        }
    }

    #[test]
    fn rejects_malformed_layouts_before_selecting_a_client() {
        let valid = mix(2, Some(3));
        assert!(valid.valid_layout(8, 384_000));
        assert!(!valid.valid_layout(4, 384_000));
        assert!(!valid.valid_layout(8, 1));
        for invalid in [
            SharedFormatInfo {
                sample_rate: 0,
                ..valid
            },
            SharedFormatInfo {
                channels: 0,
                ..valid
            },
            SharedFormatInfo {
                container_bits: 20,
                ..valid
            },
            SharedFormatInfo {
                valid_bits: Some(24),
                ..valid
            },
            SharedFormatInfo {
                channel_mask: Some(4),
                ..valid
            },
        ] {
            assert!(!invalid.valid_layout(8, 384_000));
        }
    }
}
