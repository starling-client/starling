use super::*;

#[derive(Debug, PartialEq, sunrise_mod_api_macros::Wire)]
struct Tiny {
    flag: bool,
    code: u8,
    more: Option<Box<u8>>,
}

#[derive(sunrise_mod_api_macros::Wire)]
struct Even(Option<Box<Odd>>);
#[derive(sunrise_mod_api_macros::Wire)]
struct Odd(Option<Box<Even>>);

#[test]
fn unchanged_forms_keep_the_previous_bytes_and_schema_including_mutual_recursion() {
    let value = Tiny {
        flag: true,
        code: 0xff,
        more: Some(Box::new(0xa3)),
    };
    let bytes = [1, 0xff, 1, 0xa3];
    assert_eq!(
        encode_payload(&value, PAYLOAD_DECODE_BUDGET).unwrap(),
        bytes
    );
    assert_eq!(decode_payload(&bytes), Ok(value));
    // Recorded against the former PayloadCodec layout, not calculated by Wire.
    assert_eq!(payload_schema::<Tiny>(), 0x1f02_846f_d6f3_c677);
    assert_eq!(payload_schema::<Even>(), 0x2ece_68d4_9675_316d);
    assert_eq!(payload_schema::<Odd>(), 0x2ece_68d4_9675_316d);
    let even = Even(Some(Box::new(Odd(None))));
    assert_eq!(encode_payload(&even, 2), Ok(vec![1, 0]));
    assert!(matches!(decode_payload::<Even>(&[1, 0]), Ok(Even(Some(odd))) if odd.0.is_none()));
}

#[derive(Debug, PartialEq, sunrise_mod_api_macros::Wire)]
enum Message {
    Ping,
    Data { value: u8 },
}

#[test]
fn the_canonical_enum_encoding_has_a_new_negotiated_fingerprint() {
    assert_ne!(payload_schema::<Message>(), 0x4c4c_7f32_2697_a7c3);
    let body = [1, 0, 0, 0, 7]; // fixed little-endian tag, then its one-byte field
    assert_eq!(
        encode_payload(&Message::Data { value: 7 }, 5).unwrap(),
        body
    );
    assert_eq!(decode_payload(&body), Ok(Message::Data { value: 7 }));
    assert_eq!(decode_payload::<Message>(&[1, 7]), Err(WireError::Eof));
}

#[test]
fn both_codecs_of_a_typed_body_refuse_excess_allocation_before_reserving() {
    // One byte per None in the body, but a Vec entry large enough to cross the
    // allocation budget. Exactly 4096 entries meets the distinct collection cap.
    type Wide = Vec<Option<[u8; 2048]>>;
    let mut bytes = vec![0, 0x10, 0, 0];
    bytes.resize(4 + 4096, 0);
    assert_eq!(decode_payload::<Wide>(&bytes), Err(WireError::Budget));
    assert_eq!(
        encode_payload(&vec![None::<[u8; 2048]>; 4096], PAYLOAD_DECODE_BUDGET),
        Err(WireError::Budget)
    );
}

#[test]
fn typed_caps_do_not_cap_ordinary_wire() {
    let value = "a".repeat(4097);
    let mut writer = Writer::new();
    writer.put(&value);
    assert_eq!(Reader::decode::<String>(writer.bytes()), Ok(value.clone()));
    assert_eq!(
        decode_payload::<String>(writer.bytes()),
        Err(WireError::Length)
    );
    assert_eq!(
        encode_payload(&value, PAYLOAD_DECODE_BUDGET),
        Err(WireError::Length)
    );
    assert_eq!(
        encode_payload(&"hello".to_owned(), 8),
        Err(WireError::Length)
    );
}

#[test]
fn malformed_bodies_report_the_wire_error() {
    assert_eq!(
        decode_payload::<Vec<String>>(&[1, 0x10, 0, 0]),
        Err(WireError::Length)
    );
    assert_eq!(decode_payload::<u32>(&[1, 0, 0]), Err(WireError::Eof));
    assert_eq!(
        decode_payload::<String>(&[1, 0, 0, 0, b'a', 0xaa]),
        Err(WireError::Trailing)
    );
}

#[test]
fn handwritten_recursion_keeps_its_explicit_depth_guard() {
    #[derive(Debug, PartialEq)]
    struct Explicit(u16);

    impl Wire for Explicit {
        const MIN: usize = 1;
        fn describe(_: &mut WireSchema) -> u64 {
            WireSchema::leaf("test:depth-bytes")
        }
        fn put(&self, out: &mut Writer) {
            if self.0 == 0 {
                out.put(&0u8);
            } else {
                out.put(&1u8);
                out.nested(|out| out.put(&Self(self.0 - 1)));
            }
        }
        fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
            match input.take::<u8>()? {
                0 => Ok(Self(0)),
                1 => input.nested(|input| input.take::<Self>().map(|inner| Self(inner.0 + 1))),
                tag => Err(WireError::Tag(tag.into())),
            }
        }
    }

    let mut body = vec![1; PAYLOAD_MAX_DEPTH as usize];
    body.push(0);
    assert_eq!(decode_payload(&body), Ok(Explicit(PAYLOAD_MAX_DEPTH)));
    assert_eq!(
        encode_payload(&Explicit(PAYLOAD_MAX_DEPTH), body.len()).unwrap(),
        body
    );
    body.insert(0, 1);
    assert_eq!(decode_payload::<Explicit>(&body), Err(WireError::Depth));
    assert_eq!(
        encode_payload(&Explicit(PAYLOAD_MAX_DEPTH + 1), body.len()),
        Err(WireError::Depth)
    );
}

#[test]
fn java_string_list_consumes_one_level_in_both_directions() {
    use sunrise_core::nbt::JavaString;

    #[derive(Debug, PartialEq, sunrise_mod_api_macros::Wire)]
    enum NestedKey {
        Parent(Box<NestedKey>),
        Key(JavaString),
    }

    let at_boundary = (1..PAYLOAD_MAX_DEPTH)
        .fold(NestedKey::Key(JavaString::from("a")), |child, _| {
            NestedKey::Parent(Box::new(child))
        });
    let mut bytes = vec![0; 4 * usize::from(PAYLOAD_MAX_DEPTH - 1)];
    bytes.extend([1, 0, 0, 0, 1, 0, 0, 0, b'a', 0]);
    assert_eq!(encode_payload(&at_boundary, bytes.len()).unwrap(), bytes);
    assert_eq!(
        decode_payload::<NestedKey>(&bytes).as_ref(),
        Ok(&at_boundary)
    );
    let over_boundary = NestedKey::Parent(Box::new(at_boundary));
    let bytes = [&[0; 4], bytes.as_slice()].concat();
    assert_eq!(
        encode_payload(&over_boundary, bytes.len()),
        Err(WireError::Depth)
    );
    assert_eq!(decode_payload::<NestedKey>(&bytes), Err(WireError::Depth));
}

#[test]
fn writer_resets_and_limit_raises_cannot_remove_the_payload_ceiling() {
    for reset in [
        Writer::clear,
        |out: &mut Writer| out.replace(Vec::new()),
        |out: &mut Writer| out.limit(32),
    ] {
        let mut out = Writer::for_payload(2);
        out.limit(1);
        reset(&mut out);
        out.put_bytes(&[1, 2, 3]);
        assert_eq!(out.error(), Some(WireError::Length));
        assert!(out.bytes().is_empty());
        out.clear();
        assert_eq!(out.finish_payload(), Err(WireError::Length));
    }
}

#[test]
fn writer_resets_do_not_refund_spent_payload_budget() {
    for reset in [Writer::clear, |out: &mut Writer| out.replace(Vec::new())] {
        let mut out = Writer::for_payload(PAYLOAD_DECODE_BUDGET);
        assert!(out.charge::<u8>(PAYLOAD_DECODE_BUDGET));
        reset(&mut out);
        out.put_str("a");
        assert_eq!(out.finish_payload(), Err(WireError::Budget));
    }
}

#[test]
fn arc_counts_are_charged_even_for_zero_width_values() {
    use std::sync::Arc;

    type Rows = Vec<Vec<Arc<()>>>;
    // Sharing here keeps the fixture cheap; the receiver constructs a new Arc per entry.
    let shared = Arc::new(());
    let row = vec![shared; 4096];
    // Each decoded row retains 4096 handles and two refcounts per handle: 98,304 bytes on
    // 64-bit, although the encoded row is only its 4-byte length.
    let rows = vec![row; 100];
    let mut bytes = vec![100, 0, 0, 0];
    bytes.extend([0, 0x10, 0, 0].repeat(100));
    assert_eq!(encode_payload(&rows, bytes.len()), Err(WireError::Budget));
    assert_eq!(decode_payload::<Rows>(&bytes), Err(WireError::Budget));

    assert_eq!(
        encode_payload(&rows[..2].to_vec(), 12).unwrap(),
        [2, 0, 0, 0, 0, 0x10, 0, 0, 0, 0x10, 0, 0]
    );
    assert_eq!(
        decode_payload::<Rows>(&[2, 0, 0, 0, 0, 0x10, 0, 0, 0, 0x10, 0, 0]).unwrap(),
        rows[..2]
    );
}

#[test]
fn typed_forms_with_unmetered_conversions_are_refused() {
    use crate::{MeshDefinition, ModelPart, PartPose};

    fn ordinary_only<T: Wire>(value: &T) {
        let mut writer = Writer::new();
        writer.put(value);
        assert!(Reader::decode::<T>(writer.bytes()).is_ok());
        assert!(matches!(
            encode_payload(value, PAYLOAD_DECODE_BUDGET),
            Err(WireError::Invalid(_))
        ));
        let mut input = Reader::for_payload(writer.bytes());
        assert!(matches!(T::take(&mut input), Err(WireError::Invalid(_))));
        assert_eq!(input.rest(), writer.bytes());
    }

    ordinary_only(&MeshDefinition::new());
    ordinary_only(&ModelPart::new(Vec::new(), Vec::new(), PartPose::ZERO));
    ordinary_only(&serde_json::json!({"field": []}));
    ordinary_only(&serde_json::Map::<String, serde_json::Value>::new());
    ordinary_only(&crate::LegacyRandomSource::new(42)); // a #[wire(via = ...)] conversion
    let level = crate::LevelKey::parse("test:dimension").unwrap();
    ordinary_only(&level);
    ordinary_only(&crate::LevelDimension::Custom(level));
    ordinary_only(&crate::Identifier::parse("test:item").unwrap());
    ordinary_only(&crate::SpawnerKey::new("test:spawner").unwrap());
    ordinary_only(&crate::CustomSpawners::vanilla());
    assert_eq!(
        decode_payload::<String>(&[1, 0, 0, 0, b'a']),
        Ok("a".to_owned())
    );
}

#[test]
fn sparse_maps_must_fit_their_retained_backing_storage() {
    use std::collections::{BTreeMap, HashMap};
    type WideValue = Option<[u8; 2048]>;

    // Each singleton tree map charges a node with room for eleven 2 KiB
    // values. The hash map needs only four buckets. The absent value in
    // either body needs just one tag byte.
    let mut bytes = vec![0xe0, 1, 0, 0]; // 480 singleton maps
    bytes.extend([1, 0, 0, 0, 1, 0].repeat(480));
    let hash = decode_payload::<Vec<HashMap<u8, WideValue>>>(&bytes).unwrap();
    assert_eq!(hash, vec![HashMap::from([(1, None)]); 480]);

    let tree = BTreeMap::from([(1u8, None::<[u8; 2048]>)]);
    assert_eq!(
        encode_payload(&vec![tree; 480], bytes.len()),
        Err(WireError::Budget)
    );
    assert_eq!(
        decode_payload::<Vec<BTreeMap<u8, WideValue>>>(&bytes),
        Err(WireError::Budget)
    );
}

#[test]
fn a_nearly_full_bucket_array_is_charged_at_its_rounded_size() {
    use std::collections::HashMap;
    type Rows = Vec<HashMap<u8, Option<[u8; 2048]>>>;

    // Fifteen entries require 32 buckets. Thirty would let all 128 rows fit,
    // but 32 buckets per row exceed 8 MiB; each None occupies only one wire byte.
    let mut row = HashMap::with_capacity(15);
    row.extend((0..15).map(|key| (key, None)));
    let mut rows: Rows = vec![row; 128];
    let mut entry = vec![15, 0, 0, 0];
    entry.extend((0..15).flat_map(|key| [key, 0]));
    let mut bytes = vec![128, 0, 0, 0];
    bytes.extend(entry.repeat(128));
    assert_eq!(encode_payload(&rows, bytes.len()), Err(WireError::Budget));
    assert_eq!(decode_payload::<Rows>(&bytes), Err(WireError::Budget));

    rows.pop();
    bytes[0] -= 1;
    bytes.truncate(bytes.len() - entry.len());
    assert_eq!(
        encode_payload(&rows, bytes.len()).unwrap().len(),
        bytes.len()
    );
    assert_eq!(decode_payload::<Rows>(&bytes), Ok(rows));
}

#[test]
fn a_bare_compound_counts_toward_typed_depth_even_when_empty() {
    use crate::Compound;

    #[derive(Debug, PartialEq, sunrise_mod_api_macros::Wire)]
    enum Nested {
        End(Compound),
        Next(Box<Nested>),
    }

    let links = usize::from(PAYLOAD_MAX_DEPTH - 1);
    let value = (0..links).fold(Nested::End(Compound::new()), |inner, _| {
        Nested::Next(Box::new(inner))
    });
    let mut bytes = [1, 0, 0, 0].repeat(links);
    bytes.extend([0; 8]); // the End case index and the compound length
    assert_eq!(encode_payload(&value, bytes.len()), Ok(bytes.clone()));
    assert_eq!(decode_payload::<Nested>(&bytes).as_ref(), Ok(&value));

    let over = Nested::Next(Box::new(value));
    let bytes = [&[1, 0, 0, 0], bytes.as_slice()].concat();
    assert_eq!(encode_payload(&over, bytes.len()), Err(WireError::Depth));
    assert_eq!(decode_payload::<Nested>(&bytes), Err(WireError::Depth));
}

#[test]
fn nbt_containers_share_the_remaining_typed_depth() {
    use crate::Tag;

    // An enclosing Box consumes one of the 64 levels. The 64th one is a
    // single-element NBT list; the end tag itself is not a container.
    let links = usize::from(PAYLOAD_MAX_DEPTH - 1);
    let tree = Box::new((0..links).fold(Tag::End, |inner, _| Tag::List(vec![inner])));
    let mut bytes = [9, 0, 0, 0, 1, 0, 0, 0].repeat(links);
    bytes.extend([0; 4]);
    assert_eq!(encode_payload(&tree, bytes.len()).unwrap(), bytes);
    assert_eq!(decode_payload::<Box<Tag>>(&bytes).as_ref(), Ok(&tree));

    let over = Box::new(Tag::List(vec![*tree]));
    let bytes = [&[9, 0, 0, 0, 1, 0, 0, 0], bytes.as_slice()].concat();
    assert_eq!(encode_payload(&over, bytes.len()), Err(WireError::Depth));
    assert_eq!(decode_payload::<Box<Tag>>(&bytes), Err(WireError::Depth));
    assert!(Reader::decode::<Box<Tag>>(&bytes).is_ok());
}

#[test]
fn text_trees_share_the_remaining_typed_depth_and_budget() {
    use crate::{Tag, Text};

    // An enclosing Box consumes one of the 64 levels, leaving 63 nested Text nodes.
    let mut tree = Text::literal("leaf");
    for _ in 1..usize::from(PAYLOAD_MAX_DEPTH - 1) {
        tree = Text::literal("").append(tree);
    }
    let at_limit = Box::new(tree);
    let bytes = encode_payload(&at_limit, PAYLOAD_DECODE_BUDGET).unwrap();
    assert_eq!(decode_payload::<Box<Text>>(&bytes).as_ref(), Ok(&at_limit));

    let over = Box::new(Text::literal("").append(*at_limit));
    let mut ordinary = Writer::new();
    ordinary.put(&over);
    assert_eq!(ordinary.error(), None);
    assert_eq!(
        encode_payload(&over, PAYLOAD_DECODE_BUDGET),
        Err(WireError::Depth)
    );
    assert_eq!(
        decode_payload::<Box<Text>>(ordinary.bytes()),
        Err(WireError::Depth)
    );
    assert!(Reader::decode::<Box<Text>>(ordinary.bytes()).is_ok());

    // Malformed fields still report Invalid, distinct from Depth.
    let mut bad = Text::literal("bad");
    bad.metadata = Tag::Byte(1);
    assert!(matches!(
        encode_payload(&bad, PAYLOAD_DECODE_BUDGET),
        Err(WireError::Invalid(_))
    ));

    // A wide component tree that fits the 8 MiB byte cap still trips the decode budget.
    let mut branch = Text::literal("");
    branch.children = vec![Text::literal(""); 20];
    let mut wide = Text::literal("");
    wide.children = vec![branch; 2000];
    let mut wide_bytes = Writer::new();
    wide_bytes.put(&wide);
    assert_eq!(wide_bytes.error(), None);
    assert!(wide_bytes.bytes().len() < PAYLOAD_DECODE_BUDGET);
    assert_eq!(
        encode_payload(&wide, PAYLOAD_DECODE_BUDGET),
        Err(WireError::Budget)
    );
    assert_eq!(
        decode_payload::<Text>(wide_bytes.bytes()),
        Err(WireError::Budget)
    );
}

#[test]
fn checked_decoders_do_not_share_the_raw_representations_fingerprint() {
    assert_ne!(
        payload_schema::<crate::EntityTypeKey>(),
        payload_schema::<u32>()
    );
    assert_ne!(
        payload_schema::<serde_json::Map<String, serde_json::Value>>(),
        payload_schema::<serde_json::Value>()
    );
    assert_ne!(
        payload_schema::<crate::ModelPart>(),
        payload_schema::<String>()
    );
}

#[test]
fn the_checked_entry_points_cap_the_whole_body() {
    #[derive(Debug, PartialEq)]
    struct RawBytes(usize);
    impl Wire for RawBytes {
        const MIN: usize = 0;
        fn describe(_: &mut WireSchema) -> u64 {
            WireSchema::leaf("test:raw-bytes")
        }
        fn put(&self, out: &mut Writer) {
            let block = [0; 4096];
            for _ in 0..self.0 / block.len() {
                out.put_bytes(&block);
            }
            out.put_bytes(&block[..self.0 % block.len()]);
        }
        fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
            input
                .take_bytes(input.rest().len())
                .map(|bytes| Self(bytes.len()))
        }
    }
    let limit = PAYLOAD_DECODE_BUDGET;
    let mut bytes = encode_payload(&RawBytes(limit), usize::MAX).unwrap();
    assert_eq!(decode_payload(&bytes), Ok(RawBytes(limit)));
    bytes.push(0);
    assert_eq!(
        encode_payload(&RawBytes(limit + 1), usize::MAX),
        Err(WireError::Length)
    );
    assert_eq!(decode_payload::<RawBytes>(&bytes), Err(WireError::Length));
}
