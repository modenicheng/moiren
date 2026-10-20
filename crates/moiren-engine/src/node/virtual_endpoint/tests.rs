use super::*;
use serde::de::{
    IntoDeserializer,
    value::{Error, StrDeserializer, U64Deserializer},
};

#[test]
fn endpoint_id_text_preserves_all_bits_and_byte_order() {
    let bytes = [
        0x00, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
        0x32,
    ];
    let id = VirtualEndpointId::from_bytes(bytes);
    let encoded = "000123456789abcdeffedcba98765432";
    assert_eq!(id.to_bytes(), bytes);
    assert_eq!(id.to_string(), encoded);
    for text in [encoded.to_owned(), encoded.to_uppercase()] {
        assert_eq!(text.parse::<VirtualEndpointId>().unwrap(), id);
        assert_eq!(
            VirtualEndpointId::deserialize(StrDeserializer::<Error>::new(&text)).unwrap(),
            id
        );
    }
    for bytes in [[0; 16], [0xff; 16]] {
        let id = VirtualEndpointId::from_bytes(bytes);
        assert_eq!(id.to_string().parse::<VirtualEndpointId>().unwrap(), id);
    }
    let other = VirtualEndpointId::from_bytes([
        0x80, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
        0x32,
    ]);
    assert_ne!(id, other, "identity includes the upper 64 bits");
    assert_eq!(std::mem::size_of::<VirtualEndpointId>(), 16);
}

#[test]
fn endpoint_id_rejects_truncation_non_hex_and_runtime_token_encodings() {
    for text in [
        "",
        "42",
        "000123456789abcd",
        "000123456789abcdeffedcba9876543",
        "000123456789abcdeffedcba987654321",
        "000123456789abcdeffedcba9876543g",
        "-00123456789abcdeffedcba98765432",
        "000123456789abcdeffedcba9876543 ",
        "00012345-6789-abcd-effe-dcba98765432",
    ] {
        assert_eq!(
            text.parse::<VirtualEndpointId>(),
            Err(ParseVirtualEndpointIdError),
            "{text:?}"
        );
        assert!(VirtualEndpointId::deserialize(StrDeserializer::<Error>::new(text)).is_err());
    }
    // No numeric deserialization path can reinterpret a runtime PDO token.
    let token: U64Deserializer<Error> = 42_u64.into_deserializer();
    assert!(VirtualEndpointId::deserialize(token).is_err());
}
