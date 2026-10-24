//! Explicit target exercise: retain siblings while replacing one raw endpoint.
use moiren_windows_audio::vad::{
    Channels, EndpointSnapshot, Intent, PdoState, RuntimePdoToken, StableEndpointId,
};
use std::{collections::HashSet, error::Error};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Copy)]
struct Spec {
    id: StableEndpointId,
    intent: Intent,
    channels: Channels,
}

trait Client {
    fn create(&mut self, spec: Spec) -> Result<EndpointSnapshot>;
    fn query(&mut self, previous: EndpointSnapshot) -> Result<EndpointSnapshot>;
    fn remove(&mut self, previous: EndpointSnapshot) -> Result<EndpointSnapshot>;
}

#[cfg(windows)]
impl Client for moiren_windows_audio::vad::ControlClient {
    fn create(&mut self, spec: Spec) -> Result<EndpointSnapshot> {
        let name = format!("Moiren simultaneous {:?} {:?}", spec.intent, spec.channels);
        Ok(self.create_and_wait(
            spec.id,
            spec.intent,
            spec.channels,
            &name,
            false,
            std::time::Duration::from_secs(15),
        )?)
    }
    fn query(&mut self, previous: EndpointSnapshot) -> Result<EndpointSnapshot> {
        Ok(self.query(previous.key())?)
    }
    fn remove(&mut self, previous: EndpointSnapshot) -> Result<EndpointSnapshot> {
        let closed = self.close_gate(previous)?;
        Ok(self.remove_and_wait(closed.key(), std::time::Duration::from_secs(15))?)
    }
}

#[cfg(windows)]
pub(super) fn exercise(
    client: &mut moiren_windows_audio::vad::ControlClient,
    rounds: u32,
) -> Result<()> {
    exercise_with(client, rounds)
}

fn exercise_with<C: Client>(client: &mut C, rounds: u32) -> Result<()> {
    if !(1..=10_000).contains(&rounds) {
        return Err("simultaneous rounds must be in 1..=10000".into());
    }
    let mut tokens = HashSet::new();
    for round in 1..=rounds {
        let mut live = Vec::with_capacity(4);
        for intent in [Intent::Playback, Intent::Microphone] {
            for channels in [Channels::Mono, Channels::Stereo] {
                let spec = Spec {
                    id: StableEndpointId::new(
                        intent as u64 | ((channels as u64) << 32),
                        0x4d6f_6972_656e_5349,
                    ),
                    intent,
                    channels,
                };
                let endpoint = create(client, spec, &mut tokens)?;
                live.push((spec, endpoint));
            }
        }
        check_siblings(client, &mut live)?;
        println!("round={round}/{rounds} simultaneous_present=4 gates=closed");
        for index in 0..4 {
            let (spec, previous) = live.remove(index);
            remove(client, previous)?;
            check_siblings(client, &mut live)?;
            let recreated = create(client, spec, &mut tokens)?;
            println!(
                "round={round}/{rounds} intent={:?} channels={:?} stable_id={} \
                 old_pdo={} new_pdo={} siblings_verified={}",
                spec.intent,
                spec.channels,
                spec.id,
                previous.pdo_token(),
                recreated.pdo_token(),
                live.len(),
            );
            live.insert(index, (spec, recreated));
            check_siblings(client, &mut live)?;
        }
        while let Some((_, endpoint)) = live.pop() {
            remove(client, endpoint)?;
            check_siblings(client, &mut live)?;
        }
    }
    println!(
        "Completed {rounds} simultaneous four-endpoint rounds with sibling raw-resource checks; \
         no MMDevice/KS routing or native PCM validation was performed."
    );
    Ok(())
}

fn create<C: Client>(
    client: &mut C,
    spec: Spec,
    tokens: &mut HashSet<RuntimePdoToken>,
) -> Result<EndpointSnapshot> {
    let endpoint = client.create(spec)?;
    require_ready(endpoint, spec)?;
    if !tokens.insert(endpoint.pdo_token()) {
        return Err(format!("driver reused runtime PDO token {}", endpoint.pdo_token()).into());
    }
    Ok(endpoint)
}

fn require_ready(endpoint: EndpointSnapshot, spec: Spec) -> Result<()> {
    endpoint.require_created(spec.id, spec.intent, spec.channels)?;
    if endpoint.state() != PdoState::Present
        || !endpoint.audio_ready()
        || !endpoint.policy_prepared()
    {
        return Err(
            "simultaneous endpoint is not present with ready audio and staged policy".into(),
        );
    }
    Ok(())
}

fn check_siblings<C: Client>(client: &mut C, live: &mut [(Spec, EndpointSnapshot)]) -> Result<()> {
    for (spec, previous) in live {
        let current = client.query(*previous)?;
        current
            .require_key(previous.key())?
            .require_progress_after(*previous)?;
        require_ready(current, *spec)?;
        if current.gate_epoch() != previous.gate_epoch()
            || current.stream_generation() != previous.stream_generation()
        {
            return Err(format!("sibling epoch changed during replacement: {}", spec.id).into());
        }
        *previous = current;
    }
    Ok(())
}

fn remove<C: Client>(client: &mut C, endpoint: EndpointSnapshot) -> Result<()> {
    if !client.remove(endpoint)?.removal_complete_after(endpoint)? {
        return Err("removal accepted without final Removed observation".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
