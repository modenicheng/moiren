//! Native snapshots retain their fixed limit; control archives select a
//! separate bounded limit. Never trust an unbounded caller sequence size hint.
use serde::{Deserialize, Deserializer, de};
use std::{fmt, marker::PhantomData};

pub(super) fn deserialize_events<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    deserialize_bounded_events::<D, T, 256>(deserializer)
}

pub(crate) fn deserialize_bounded_events<'de, D, T, const LIMIT: usize>(
    deserializer: D,
) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Events<T, const LIMIT: usize>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const LIMIT: usize> de::Visitor<'de> for Events<T, LIMIT> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            write!(formatter, "at most {LIMIT} boundary events")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Vec<T>, A::Error> {
            let mut events = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(LIMIT));
            while let Some(event) = sequence.next_element()? {
                if events.len() == LIMIT {
                    return Err(de::Error::custom("boundary history exceeds event limit"));
                }
                events.push(event);
            }
            Ok(events)
        }
    }
    deserializer.deserialize_seq(Events::<T, LIMIT>(PhantomData))
}
