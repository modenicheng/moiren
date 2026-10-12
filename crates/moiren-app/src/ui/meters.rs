use super::signals::{PresentationDemand, PresentationRect, VisibleSignal};
use super::{Backend, MainWindow, MeterRegion, SignalRect, WindowPresentation};
use crate::service::{
    AppHandle, AppSnapshot, InputSelection, LevelTopic, SessionGeneration, SessionPhase,
};
use moiren_engine::meter::signal::{AudioWindow, ChannelLevel};
use moiren_signal::{PublisherStatus, Subscriber, TopicLifecycle};
use slint::{ComponentHandle, Model};

#[derive(Default)]
struct Reading {
    levels: Option<[ChannelLevel; 2]>,
    window: Option<AudioWindow>,
    status: String,
}

impl Reading {
    fn read(
        &mut self,
        signal: &mut VisibleSignal<LevelTopic>,
        demand: PresentationDemand,
        after_sequence: u64,
    ) {
        if !demand.needs_subscription() {
            *self = Self {
                status: "Levels paused while hidden".into(),
                ..Self::default()
            };
        }
        if let Err(error) = signal.update_demand(demand) {
            self.status = format!("Subscription unavailable: {error:?}");
            return;
        }
        if !demand.needs_subscription() {
            return;
        }
        let subscriber = match signal.subscriber() {
            Ok(Some(subscriber)) => subscriber,
            Ok(None) => {
                self.status = "Waiting for a subscription".into();
                return;
            }
            Err(error) => {
                self.status = format!("Subscription unavailable: {error}");
                return;
            }
        };
        self.read_subscriber(subscriber, after_sequence);
    }

    fn read_subscriber(&mut self, subscriber: &mut Subscriber<LevelTopic>, after_sequence: u64) {
        let status = subscriber.status();
        if status.lifecycle == TopicLifecycle::Closed {
            *self = Self {
                status: "Signal topic closed".into(),
                ..Self::default()
            };
            return;
        }
        if status.publisher == PublisherStatus::NoPublisher {
            *self = Self {
                status: "No publisher".into(),
                ..Self::default()
            };
            return;
        }
        if let Some(message) = subscriber.read_latest() {
            if message.meta().sequence <= after_sequence {
                *self = Self {
                    status: "Waiting for a complete audio window".into(),
                    ..Self::default()
                };
                return;
            }
            if let Ok(levels) = message.elements().try_into() {
                self.levels = Some(levels);
                self.window = Some(*message.header());
                self.status = "Live · peak / RMS".into();
            } else {
                *self = Self {
                    status: "Unsupported channel layout".into(),
                    ..Self::default()
                };
            }
        } else if self.levels.is_none() {
            self.status = "Waiting for a complete audio window".into();
        }
    }

    fn bars(&self) -> [f32; 4] {
        self.levels.map_or([0.0; 4], |[left, right]| {
            [
                meter_position(left.rms),
                meter_position(right.rms),
                meter_position(left.peak),
                meter_position(right.peak),
            ]
        })
    }

    fn labels(&self) -> String {
        self.levels.map_or_else(
            || "L — / —   R — / — dB".into(),
            |[left, right]| {
                format!(
                    "L {} / {} dB\nR {} / {} dB",
                    db_label(left.peak),
                    db_label(left.rms),
                    db_label(right.peak),
                    db_label(right.rms)
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::SignalPreparation;
    use crate::tone::{ToneConfig, ToneSession, prepare_tone_with_signals};
    use moiren_signal::{BlockInit, Bus, BusConfig, PumpBudget};

    fn render_window(tone: &mut ToneSession) {
        for _ in 0..6 {
            tone.compiled.engine.render(256).unwrap();
            tone.output.read_interleaved(&mut [0.0; 512]).unwrap();
        }
    }

    #[test]
    fn restarted_source_does_not_present_cached_previous_session_levels() {
        let mut bus = Bus::new(BusConfig {
            max_topics: 1,
            ..BusConfig::default()
        })
        .unwrap();
        let topic = bus
            .register_latest_block(
                "session/output/level",
                BlockInit {
                    header: AudioWindow {
                        timeline_epoch: 0,
                        frame_start: 0,
                        frame_end: 0,
                        sample_rate: 0.0,
                    },
                    element: ChannelLevel {
                        peak: 0.0,
                        rms: 0.0,
                    },
                    capacity: 2,
                },
            )
            .unwrap();
        let _analysis = bus.subscribe(&topic, Default::default()).unwrap();
        let mut preparation = SignalPreparation::new(&mut bus, None, topic.clone()).unwrap();
        let mut tone = prepare_tone_with_signals(ToneConfig::default(), &mut preparation).unwrap();
        let previous_epoch = tone.compiled.engine.epoch();
        render_window(&mut tone);
        bus.pump(PumpBudget {
            topic_checks: 32,
            deliveries: 64,
            copy_bytes: 256 * 1024,
        });
        drop(tone);
        drop(preparation);
        let after_sequence = topic.stats().attempts;
        let mut preparation = SignalPreparation::new(&mut bus, None, topic.clone()).unwrap();
        let mut tone = prepare_tone_with_signals(ToneConfig::default(), &mut preparation).unwrap();
        tone.compiled.engine.render(256).unwrap();
        tone.output.read_interleaved(&mut [0.0; 512]).unwrap();
        assert_eq!(topic.status().publisher, PublisherStatus::Bound);
        let mut cached_witness = bus.subscribe(&topic, Default::default()).unwrap();
        assert_eq!(
            cached_witness
                .read_latest()
                .unwrap()
                .header()
                .timeline_epoch,
            previous_epoch
        );
        let mut ui = bus.subscribe(&topic, Default::default()).unwrap();
        let mut reading = Reading::default();
        reading.read_subscriber(&mut ui, after_sequence);
        assert!(
            reading.levels.is_none(),
            "previous session cache is not live audio"
        );
        assert!(reading.window.is_none());
        render_window(&mut tone);
        assert!(topic.stats().attempts > after_sequence);
        bus.pump(PumpBudget {
            topic_checks: 32,
            deliveries: 64,
            copy_bytes: 256 * 1024,
        });
        reading.read_subscriber(&mut ui, after_sequence);
        assert!(reading.levels.is_some());
        assert_eq!(
            reading.window.unwrap().timeline_epoch,
            tone.compiled.engine.epoch()
        );
        assert_ne!(reading.window.unwrap().timeline_epoch, previous_epoch);
    }
}

fn meter_position(amplitude: f32) -> f32 {
    if amplitude.is_finite() && amplitude > 0.0 {
        ((20.0 * amplitude.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn db_label(amplitude: f32) -> String {
    if amplitude.is_finite() && amplitude > 0.0 {
        format!("{:.1}", 20.0 * amplitude.log10())
    } else {
        "−∞".into()
    }
}

#[derive(Default)]
pub(super) struct UiSignals {
    session: Option<(SessionGeneration, bool)>,
    stopped_generation: Option<SessionGeneration>,
    output: Option<VisibleSignal<LevelTopic>>,
    input: Option<VisibleSignal<LevelTopic>>,
    output_after_sequence: u64,
    input_after_sequence: u64,
    output_reading: Reading,
    input_reading: Reading,
}

impl UiSignals {
    pub fn hide(&mut self) {
        self.session = None;
        self.output = None;
        self.input = None;
        self.output_after_sequence = 0;
        self.input_after_sequence = 0;
        self.output_reading = Reading::default();
        self.input_reading = Reading::default();
    }

    pub fn stop(&mut self, generation: SessionGeneration) {
        self.hide();
        self.stopped_generation = Some(generation);
    }

    pub fn update(
        &mut self,
        handle: &AppHandle,
        snapshot: &AppSnapshot,
        window: &MainWindow,
        presentation: &WindowPresentation,
    ) -> MeterDisplay {
        if snapshot.phase != SessionPhase::Running
            || self.stopped_generation == Some(snapshot.generation)
        {
            self.hide();
            return self.display(snapshot, window);
        }
        let Some(spec) = snapshot.desired.as_ref() else {
            self.hide();
            return self.display(snapshot, window);
        };
        let tone = matches!(spec.source, InputSelection::Tone { .. });
        if self.session != Some((snapshot.generation, tone)) {
            self.hide();
            self.stopped_generation = None;
            let topic = if tone {
                snapshot.signal_topics.tone_output.clone()
            } else {
                snapshot.signal_topics.monitor_output.clone()
            };
            self.output_after_sequence = topic.stats().attempts;
            self.output = Some(VisibleSignal::new(
                handle.clone(),
                topic,
                Default::default(),
            ));
            if !tone {
                self.input_after_sequence = snapshot.signal_topics.monitor_input.stats().attempts;
                self.input = Some(VisibleSignal::new(
                    handle.clone(),
                    snapshot.signal_topics.monitor_input.clone(),
                    Default::default(),
                ));
            }
            self.session = Some((snapshot.generation, tone));
        }
        let output_demand = demand(window, presentation, window.get_output_meter_regions());
        let input_demand = demand(window, presentation, window.get_input_meter_regions());
        if let Some(signal) = self.output.as_mut() {
            self.output_reading
                .read(signal, output_demand, self.output_after_sequence);
        }
        if let Some(signal) = self.input.as_mut() {
            self.input_reading
                .read(signal, input_demand, self.input_after_sequence);
        } else {
            self.input_reading.status = "Test tone has no input publisher".into();
        }
        self.display(snapshot, window)
    }

    fn display(&self, snapshot: &AppSnapshot, window: &MainWindow) -> MeterDisplay {
        MeterDisplay {
            output: self.output_reading.bars(),
            input: self.input_reading.bars(),
            output_live: self.output_reading.levels.is_some(),
            input_live: self.input_reading.levels.is_some(),
            output_label: self.output_reading.labels(),
            input_label: self.input_reading.labels(),
            output_status: if self.output_reading.status.is_empty() {
                "Start a session to read levels".into()
            } else {
                self.output_reading.status.clone()
            },
            input_status: if self.input_reading.status.is_empty() {
                "No input publisher".into()
            } else {
                self.input_reading.status.clone()
            },
            debug: self.debug(snapshot, window),
        }
    }

    fn debug(&self, snapshot: &AppSnapshot, window: &MainWindow) -> String {
        #[cfg(debug_assertions)]
        {
            if std::env::var_os("MOIREN_SIGNAL_DEBUG").is_none() {
                return String::new();
            }
            let topics = &snapshot.signal_topics;
            let stats = [&topics.monitor_input, &topics.monitor_output, &topics.tone_output].map(|topic| {
                let demand = topic.demand_snapshot();
                serde_json::json!({"subscribers": topic.business_subscribers(), "generation": demand.generation, "attempts": topic.stats().attempts})
            });
            serde_json::json!({
                "topics": stats,
                "output": self.output_reading.levels.map(|levels| levels.map(|level| [level.peak, level.rms])),
                "input": self.input_reading.levels.map(|levels| levels.map(|level| [level.peak, level.rms])),
                "output_regions": window.get_output_meter_regions().iter().map(|region| serde_json::json!({
                    "enabled":region.enabled,
                    "bounds":[region.bounds.left,region.bounds.top,region.bounds.width,region.bounds.height],
                    "viewport":[region.viewport.left,region.viewport.top,region.viewport.width,region.viewport.height],
                })).collect::<Vec<_>>(),
            }).to_string()
        }
        #[cfg(not(debug_assertions))]
        {
            let _ = (snapshot, window);
            String::new()
        }
    }
}

pub(super) struct MeterDisplay {
    output: [f32; 4],
    input: [f32; 4],
    output_live: bool,
    input_live: bool,
    output_label: String,
    input_label: String,
    output_status: String,
    input_status: String,
    debug: String,
}

impl MeterDisplay {
    pub fn apply(self, window: &MainWindow) {
        let backend = window.global::<Backend>();
        let [left, right, left_peak, right_peak] = self.output;
        backend.set_route_meter_l(left);
        backend.set_route_meter_r(right);
        backend.set_route_peak_l(left_peak);
        backend.set_route_peak_r(right_peak);
        backend.set_output_meter_live(self.output_live);
        backend.set_meter_status(self.output_status.into());
        backend.set_output_levels(self.output_label.into());
        let [left, right, left_peak, right_peak] = self.input;
        backend.set_input_meter_l(left);
        backend.set_input_meter_r(right);
        backend.set_input_peak_l(left_peak);
        backend.set_input_peak_r(right_peak);
        backend.set_input_meter_live(self.input_live);
        backend.set_input_meter_status(self.input_status.into());
        backend.set_input_levels(self.input_label.into());
        backend.set_signal_debug(self.debug.into());
    }
}

fn rect(value: SignalRect) -> PresentationRect {
    PresentationRect {
        left: value.left,
        top: value.top,
        width: value.width,
        height: value.height,
    }
}

fn demand(
    window: &MainWindow,
    presentation: &WindowPresentation,
    regions: slint::ModelRc<MeterRegion>,
) -> PresentationDemand {
    let body = rect(window.get_meter_body_clip());
    let window_clip = PresentationRect {
        left: 0.0,
        top: 0.0,
        width: window.get_shell_width(),
        height: window.get_shell_height(),
    };
    let component_visible = regions.iter().any(|region| region.enabled);
    let intersects_viewport = regions.iter().any(|region| {
        region.enabled
            && rect(region.bounds).intersects_viewport(rect(region.viewport), &[body, window_clip])
    });
    PresentationDemand {
        window_visible: presentation.visible,
        minimized: presentation.minimized,
        page_active: matches!(presentation.page, 0 | 1),
        component_visible,
        intersects_viewport,
    }
}

pub(super) fn clear_backend(window: &MainWindow) {
    let backend = window.global::<Backend>();
    backend.set_output_meter_live(false);
    backend.set_input_meter_live(false);
    backend.set_route_meter_l(0.0);
    backend.set_route_meter_r(0.0);
    backend.set_route_peak_l(0.0);
    backend.set_route_peak_r(0.0);
    backend.set_input_meter_l(0.0);
    backend.set_input_meter_r(0.0);
    backend.set_input_peak_l(0.0);
    backend.set_input_peak_r(0.0);
    backend.set_output_levels("L — / —   R — / — dB".into());
    backend.set_input_levels("L — / —   R — / — dB".into());
    backend.set_meter_status("Start a session to read levels".into());
    backend.set_input_meter_status("No input publisher".into());
}
