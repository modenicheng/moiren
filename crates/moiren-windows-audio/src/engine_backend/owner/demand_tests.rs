use super::*;
use moiren_engine::audio_config::SampleFormat;

struct DemandStream {
    demand: usize,
    written: usize,
}

impl NativeStream for DemandStream {
    fn info(&self) -> NativeStreamInfo {
        NativeStreamInfo {
            sample_rate: 48_000,
            format: SampleFormat::Float32,
            buffer_frames: 256,
            channels: 2,
        }
    }
    fn start(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn stop(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    fn wait(&mut self, _: Duration) -> Result<bool, BackendError> {
        Ok(true)
    }
    fn read(&mut self, _: &mut [StereoFrame]) -> Result<usize, BackendError> {
        Ok(0)
    }
    fn render_demand(&mut self) -> Result<usize, BackendError> {
        Ok(self.demand)
    }
    fn write(&mut self, input: &[StereoFrame]) -> Result<usize, BackendError> {
        self.written += input.len();
        Ok(input.len())
    }
}

#[test]
fn pump_renders_current_demand_without_prefetching_native_capacity() {
    let (_tx, rx) = rtrb::RingBuffer::new(1024);
    let counters = Arc::new(Counters::default());
    let mut stream = DemandStream {
        demand: 32,
        written: 0,
    };
    let mut pump = Pump::new(
        None,
        None,
        Some(rx),
        stream.info(),
        48_000,
        1024,
        counters.clone(),
        false,
    )
    .unwrap();
    let timeline = Timeline::default();
    assert!(pump.cycle::<f32>(&mut stream, None, &timeline).unwrap());
    assert_eq!(stream.written, 32);
    assert_eq!(counters.silent.load(Ordering::Relaxed), 32);

    stream.demand = 0;
    assert!(!pump.cycle::<f32>(&mut stream, None, &timeline).unwrap());
    assert_eq!(stream.written, 32);
    assert_eq!(counters.silent.load(Ordering::Relaxed), 32);

    stream.demand = 257;
    assert!(pump.cycle::<f32>(&mut stream, None, &timeline).is_err());
    assert_eq!(stream.written, 32);
}
