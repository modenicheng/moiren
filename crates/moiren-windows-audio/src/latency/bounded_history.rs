//! Import bounds match the fixed native histories; parsing never trusts a
//! caller-provided sequence size hint to allocate an unbounded event vector.
use serde::{Deserialize, Deserializer, de};
use std::{fmt, marker::PhantomData};

pub(super) fn deserialize_events<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Events<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> de::Visitor<'de> for Events<T> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("at most 256 boundary events")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Vec<T>, A::Error> {
            let mut events = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(256));
            while let Some(event) = sequence.next_element()? {
                if events.len() == 256 {
                    return Err(de::Error::custom("boundary history exceeds 256 events"));
                }
                events.push(event);
            }
            Ok(events)
        }
    }
    deserializer.deserialize_seq(Events(PhantomData))
}
