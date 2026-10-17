use super::*;

mod prepare;
mod validation;

fn slot(id: u32, processor: RackProcessor) -> RackSlot {
    RackSlot {
        id,
        bypass: false,
        processor,
    }
}

fn rack(processor: RackProcessor) -> RackSettings {
    RackSettings {
        slots: vec![slot(0, processor)],
    }
}

fn processors() -> [RackProcessor; 8] {
    [
        RackProcessor::Gain { initial: 0.5 },
        RackProcessor::Pan {
            initial: 0.25,
            mono: true,
            swap: true,
        },
        RackProcessor::Compressor(CompressorSettings::default()),
        RackProcessor::Eq(EqSettings::default()),
        RackProcessor::Gate(GateSettings::default()),
        RackProcessor::Limiter(LimiterSettings::default()),
        RackProcessor::Delay(DelaySettings::default()),
        RackProcessor::ChannelMapper(ChannelMapperSettings::default()),
    ]
}
