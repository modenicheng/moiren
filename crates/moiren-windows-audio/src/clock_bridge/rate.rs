//! A conservative capture/QPC slope estimate, independent of callback arrival.

const WINDOW_QPC_100NS: u64 = 10_000_000;
const MINIMUM_PACKETS: u32 = 8;
const MAXIMUM_RATE_PPM: i64 = 2_000;
const WINDOW_AGREEMENT_PPM: i64 = 100;

#[derive(Clone, Copy)]
struct RateAnchor {
    position: u64,
    qpc_100ns: u64,
}

#[derive(Clone, Copy)]
struct RateWindow {
    frames: u64,
    elapsed_100ns: u64,
    ppm: i64,
}

#[derive(Default)]
pub(super) struct RateClock {
    anchor: Option<RateAnchor>,
    packets: u32,
    previous_window: Option<RateWindow>,
    estimate: Option<i64>,
}

fn correction_ppm(frames: u64, elapsed_100ns: u64, sample_rate: u32) -> Option<i64> {
    let observed = i128::from(frames) * 10_000_000;
    let nominal = i128::from(elapsed_100ns) * i128::from(sample_rate);
    let difference = (observed - nominal) * 1_000_000;
    let magnitude = (difference.abs() + nominal / 2) / nominal;
    let signed = if difference < 0 {
        -magnitude
    } else {
        magnitude
    };
    i64::try_from(signed).ok()
}

impl RateClock {
    pub fn observe(&mut self, position: u64, qpc_100ns: u64, sample_rate: u32) -> Option<i64> {
        let current = RateAnchor {
            position,
            qpc_100ns,
        };
        let anchor = *self.anchor.get_or_insert(current);
        self.packets = self.packets.saturating_add(1);
        // PacketTiming already enforces monotonic, contiguous anchors. Frames
        // and QPC are differenced before division; large absolute QPC values
        // therefore do not lose precision in a floating-point conversion.
        let frames = position - anchor.position;
        let elapsed_100ns = qpc_100ns - anchor.qpc_100ns;
        if elapsed_100ns < WINDOW_QPC_100NS || self.packets < MINIMUM_PACKETS {
            return self.estimate;
        }
        let Some(ppm) = correction_ppm(frames, elapsed_100ns, sample_rate)
            .filter(|ppm| ppm.abs() <= MAXIMUM_RATE_PPM)
        else {
            self.anchor = Some(current);
            self.packets = 0;
            self.previous_window = None;
            self.estimate = None;
            return None;
        };
        let window = RateWindow {
            frames,
            elapsed_100ns,
            ppm,
        };
        self.anchor = Some(current);
        self.packets = 0;
        self.estimate = self.previous_window.and_then(|previous| {
            if (window.ppm - previous.ppm).abs() <= WINDOW_AGREEMENT_PPM {
                correction_ppm(
                    previous.frames.checked_add(window.frames)?,
                    previous.elapsed_100ns.checked_add(window.elapsed_100ns)?,
                    sample_rate,
                )
            } else {
                None
            }
        });
        self.previous_window = Some(window);
        self.estimate
    }
}
