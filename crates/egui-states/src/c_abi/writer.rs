//! Encoder for values passed to the server.

use bytes::Bytes;
use serde::Serialize;

use super::error::{Error, Result};
use super::kinds::{DataKind, ValueKind};
use super::tree::{Cursor, Node, TypeRef, take_len, walk};

/// One write, recorded so a commit can check it against the target type.
#[derive(Clone, Copy, Debug)]
enum Op {
    Num(DataKind),
    Bool,
    String,
    None,
    Some,
    Len(u64),
    Enum(u32),
    Array(DataKind, u64),
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::Num(kind) | Op::Array(kind, _) => kind.name(),
            Op::Bool => "bool",
            Op::String => "String",
            Op::None => "None",
            Op::Some => "Some",
            Op::Len(_) => "length",
            Op::Enum(_) => "enum variant",
        }
    }
}

/// Reusable encoder for one value.
///
/// Values are written in declaration order: struct fields and tuple elements
/// one after another, `Option` as [`Self::none`] or [`Self::some`] followed by
/// the value, `Vec` and `HashMap` as [`Self::count`] followed by the elements
/// or key/value pairs, and enums as their variant index. Every commit checks
/// the recorded writes against the target's type; committing does not change
/// the writer.
#[derive(Clone, Debug, Default)]
pub struct Writer {
    bytes: Vec<u8>,
    ops: Vec<Op>,
}

impl Writer {
    /// Creates an empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Removes everything written so far.
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.ops.clear();
    }

    fn encode<T: Serialize + ?Sized>(&mut self, value: &T, op: Op) -> Result<()> {
        let bytes = std::mem::take(&mut self.bytes);
        self.bytes = postcard::to_extend(value, bytes)
            .map_err(|error| Error::internal(format!("encoding failed: {error}")))?;
        self.ops.push(op);
        Ok(())
    }

    /// Writes a `u8`.
    pub fn u8(&mut self, value: u8) -> Result<()> {
        self.encode(&value, Op::Num(DataKind::U8))
    }

    /// Writes a `u16`.
    pub fn u16(&mut self, value: u16) -> Result<()> {
        self.encode(&value, Op::Num(DataKind::U16))
    }

    /// Writes a `u32`.
    pub fn u32(&mut self, value: u32) -> Result<()> {
        self.encode(&value, Op::Num(DataKind::U32))
    }

    /// Writes a `u64`.
    pub fn u64(&mut self, value: u64) -> Result<()> {
        self.encode(&value, Op::Num(DataKind::U64))
    }

    /// Writes an `i8`.
    pub fn i8(&mut self, value: i8) -> Result<()> {
        self.encode(&value, Op::Num(DataKind::I8))
    }

    /// Writes an `i16`.
    pub fn i16(&mut self, value: i16) -> Result<()> {
        self.encode(&value, Op::Num(DataKind::I16))
    }

    /// Writes an `i32`.
    pub fn i32(&mut self, value: i32) -> Result<()> {
        self.encode(&value, Op::Num(DataKind::I32))
    }

    /// Writes an `i64`.
    pub fn i64(&mut self, value: i64) -> Result<()> {
        self.encode(&value, Op::Num(DataKind::I64))
    }

    /// Writes an `f32`.
    pub fn f32(&mut self, value: f32) -> Result<()> {
        self.encode(&value, Op::Num(DataKind::F32))
    }

    /// Writes an `f64`.
    pub fn f64(&mut self, value: f64) -> Result<()> {
        self.encode(&value, Op::Num(DataKind::F64))
    }

    /// Writes a `bool`.
    pub fn bool(&mut self, value: bool) -> Result<()> {
        self.encode(&value, Op::Bool)
    }

    /// Writes a `String`.
    pub fn string(&mut self, value: &str) -> Result<()> {
        self.encode(value, Op::String)
    }

    /// Writes an absent `Option`.
    pub fn none(&mut self) -> Result<()> {
        self.encode(&0u8, Op::None)
    }

    /// Writes a present `Option`; its value follows.
    pub fn some(&mut self) -> Result<()> {
        self.encode(&1u8, Op::Some)
    }

    /// Writes the element count of a `Vec` or the entry count of a `HashMap`;
    /// the elements or key/value pairs follow.
    pub fn count(&mut self, count: u64) -> Result<()> {
        self.encode(&count, Op::Len(count))
    }

    /// Writes an enum as its variant index in declaration order (not its
    /// discriminant).
    pub fn enum_index(&mut self, index: u32) -> Result<()> {
        self.encode(&index, Op::Enum(index))
    }

    /// Writes `count` numbers of `kind` from native-endian `raw` bytes, as if
    /// each was written on its own.
    pub fn array(&mut self, kind: DataKind, raw: &[u8], count: usize) -> Result<()> {
        if count.checked_mul(kind.size()) != Some(raw.len()) {
            return Err(Error::invalid_argument(format!(
                "{} bytes do not hold {count} {} values",
                raw.len(),
                kind.name()
            )));
        }
        let start = (self.bytes.len(), self.ops.len());
        let result = raw
            .chunks_exact(kind.size())
            .try_for_each(|chunk| self.encode_raw(kind, chunk));
        if result.is_err() {
            self.bytes.truncate(start.0);
        }
        self.ops.truncate(start.1);
        result?;
        self.ops.push(Op::Array(kind, count as u64));
        Ok(())
    }

    fn encode_raw(&mut self, kind: DataKind, chunk: &[u8]) -> Result<()> {
        fn array<const N: usize>(chunk: &[u8]) -> [u8; N] {
            chunk.try_into().expect("chunk has the element size")
        }
        match kind {
            DataKind::U8 => self.u8(u8::from_ne_bytes(array(chunk))),
            DataKind::U16 => self.u16(u16::from_ne_bytes(array(chunk))),
            DataKind::U32 => self.u32(u32::from_ne_bytes(array(chunk))),
            DataKind::U64 => self.u64(u64::from_ne_bytes(array(chunk))),
            DataKind::I8 => self.i8(i8::from_ne_bytes(array(chunk))),
            DataKind::I16 => self.i16(i16::from_ne_bytes(array(chunk))),
            DataKind::I32 => self.i32(i32::from_ne_bytes(array(chunk))),
            DataKind::I64 => self.i64(i64::from_ne_bytes(array(chunk))),
            DataKind::F32 => self.f32(f32::from_ne_bytes(array(chunk))),
            DataKind::F64 => self.f64(f64::from_ne_bytes(array(chunk))),
        }
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Checks the recorded writes against `target`, then the encoded bytes.
    pub(crate) fn validate(&self, target: &TypeRef) -> Result<()> {
        let tree = &target.tree;
        let mut cursor = Cursor::new(target.node);
        for (index, op) in self.ops.iter().enumerate() {
            let repeat = match *op {
                Op::Array(kind, count) => (Op::Num(kind), count),
                op => (op, 1),
            };
            for _ in 0..repeat.1 {
                expect(&mut cursor, target, repeat.0, index)?;
            }
        }
        if let Some(node) = cursor.next_leaf(tree) {
            return Err(Error::type_mismatch(format!(
                "missing {} at {} after {} writes",
                tree.kind(node).name(),
                cursor.path(tree),
                self.ops.len()
            )));
        }

        let end = walk(tree, target.node, &self.bytes, 0)?;
        if end != self.bytes.len() {
            return Err(Error::internal("encoded value has trailing bytes"));
        }
        Ok(())
    }

    /// Checks the writes against `target` and copies the encoded value.
    pub(crate) fn encoded(&self, target: &TypeRef) -> Result<Bytes> {
        self.validate(target)?;
        Ok(Bytes::copy_from_slice(&self.bytes))
    }

    /// Checks a whole `Vec` against `whole` and splits it into its elements.
    pub(crate) fn items(&self, whole: &TypeRef) -> Result<Vec<Bytes>> {
        let bytes = self.encoded(whole)?;
        let Node::Vec(element) = whole.tree.node(whole.node) else {
            return Err(Error::internal("vector type expected"));
        };
        let (len, mut pos) = take_len(&whole.tree, element, &bytes, 0)?;
        if whole.tree.is_zero_sized(element) {
            return Ok(vec![Bytes::new(); len as usize]);
        }
        let mut items = Vec::with_capacity(len as usize);
        for _ in 0..len {
            let start = pos;
            pos = walk(&whole.tree, element, &bytes, pos)?;
            items.push(bytes.slice(start..pos));
        }
        Ok(items)
    }

    /// Checks a whole `HashMap` against `whole` and splits it into its entries.
    pub(crate) fn entries(&self, whole: &TypeRef) -> Result<Vec<(Bytes, Bytes)>> {
        let bytes = self.encoded(whole)?;
        let Node::Map(key, value) = whole.tree.node(whole.node) else {
            return Err(Error::internal("map type expected"));
        };
        let (len, mut pos) = take_len(&whole.tree, key, &bytes, 0)?;
        let mut entries = Vec::with_capacity(len.min(bytes.len() as u64) as usize);
        for _ in 0..len {
            let start = pos;
            pos = walk(&whole.tree, key, &bytes, pos)?;
            let key_bytes = bytes.slice(start..pos);
            let start = pos;
            pos = walk(&whole.tree, value, &bytes, pos)?;
            entries.push((key_bytes, bytes.slice(start..pos)));
        }
        Ok(entries)
    }
}

fn expect(cursor: &mut Cursor, target: &TypeRef, op: Op, index: usize) -> Result<()> {
    let tree = &target.tree;
    let Some(node) = cursor.next_leaf(tree) else {
        return Err(Error::type_mismatch(format!(
            "write #{index} ({}) comes after the end of the value",
            op.name()
        )));
    };
    let mismatch = |expected: ValueKind| {
        Error::type_mismatch(format!(
            "write #{index} ({}) does not match {} expected at {}",
            op.name(),
            expected.name(),
            cursor.path(tree)
        ))
    };
    match (tree.node(node), op) {
        (Node::Num(expected), Op::Num(kind)) if expected == kind => Ok(()),
        (Node::Bool, Op::Bool) | (Node::String, Op::String) | (Node::Option(_), Op::None) => Ok(()),
        (Node::Option(inner), Op::Some) => {
            cursor.open_some(inner);
            Ok(())
        }
        (Node::Vec(element), Op::Len(len)) => cursor.open_vec(tree, element, len),
        (Node::Map(key, value), Op::Len(len)) => cursor.open_map(tree, key, value, len),
        (Node::Enum { variants }, Op::Enum(variant)) => {
            if variant < variants {
                Ok(())
            } else {
                Err(Error::type_mismatch(format!(
                    "write #{index}: variant index {variant} is out of range for an enum with \
                     {variants} variants at {}",
                    cursor.path(tree)
                )))
            }
        }
        _ => Err(mismatch(tree.kind(node))),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::Typed;
    use crate::c_abi::ErrorCode;

    #[crate::typed]
    #[derive(Debug, PartialEq)]
    enum Sparse {
        Low = -7,
        High = 201,
    }

    #[crate::typed]
    #[derive(Debug, PartialEq)]
    struct Packet {
        signed: i64,
        unsigned: u64,
        text: String,
        optional: Option<i32>,
        fixed: [u16; 3],
        choice: Sparse,
        unit: (),
        list: Vec<(u8, bool)>,
        floats: (f32, f64),
        small: (i8, i16, u16, u32),
    }

    fn packet() -> Packet {
        Packet {
            signed: i64::MIN,
            unsigned: u64::MAX,
            text: "žluťoučký 🦀".into(),
            optional: Some(i32::MIN),
            fixed: [0, 32768, 65535],
            choice: Sparse::High,
            unit: (),
            list: vec![(1, true), (255, false)],
            floats: (-0.0, f64::NAN),
            small: (-128, -300, 65535, 70000),
        }
    }

    fn write_packet(writer: &mut Writer, value: &Packet) {
        writer.i64(value.signed).unwrap();
        writer.u64(value.unsigned).unwrap();
        writer.string(&value.text).unwrap();
        match value.optional {
            Some(inner) => {
                writer.some().unwrap();
                writer.i32(inner).unwrap();
            }
            None => writer.none().unwrap(),
        }
        let fixed: Vec<u8> = value.fixed.iter().flat_map(|v| v.to_ne_bytes()).collect();
        writer.array(DataKind::U16, &fixed, 3).unwrap();
        writer.enum_index(1).unwrap();
        writer.count(value.list.len() as u64).unwrap();
        for (number, flag) in &value.list {
            writer.u8(*number).unwrap();
            writer.bool(*flag).unwrap();
        }
        writer.f32(value.floats.0).unwrap();
        writer.f64(value.floats.1).unwrap();
        writer.i8(value.small.0).unwrap();
        writer.i16(value.small.1).unwrap();
        writer.u16(value.small.2).unwrap();
        writer.u32(value.small.3).unwrap();
    }

    #[test]
    fn writes_encode_like_serde() {
        let value = packet();
        let mut writer = Writer::new();
        write_packet(&mut writer, &value);
        assert_eq!(writer.bytes(), postcard::to_stdvec(&value).unwrap());
        writer.validate(&TypeRef::new(&Packet::get_type())).unwrap();

        let map: HashMap<String, Option<Sparse>> = [("a".to_owned(), Some(Sparse::Low))].into();
        let mut writer = Writer::new();
        writer.count(1).unwrap();
        writer.string("a").unwrap();
        writer.some().unwrap();
        writer.enum_index(0).unwrap();
        assert_eq!(writer.bytes(), postcard::to_stdvec(&map).unwrap());
        writer
            .validate(&TypeRef::new(&<HashMap<String, Option<Sparse>>>::get_type()))
            .unwrap();
    }

    fn mismatch<T: Typed>(write: impl FnOnce(&mut Writer), message: &str) {
        let mut writer = Writer::new();
        write(&mut writer);
        let error = writer.validate(&TypeRef::new(&T::get_type())).unwrap_err();
        assert_eq!(error.code, ErrorCode::TypeMismatch, "{}", error.message);
        assert!(error.message.contains(message), "{}", error.message);
    }

    #[test]
    fn validation_reports_type_confusion() {
        // Same varint bytes, different meaning: only the op log catches it.
        mismatch::<i32>(|w| w.u32(5).unwrap(), "write #0 (u32) does not match i32");
        mismatch::<(u8, String)>(|w| w.u8(1).unwrap(), "missing String at .1");
        mismatch::<u8>(
            |w| {
                w.u8(1).unwrap();
                w.u8(2).unwrap();
            },
            "comes after the end of the value",
        );
        mismatch::<Vec<u8>>(|w| w.count(2).unwrap(), "missing u8 at [0]");
        mismatch::<Option<u8>>(|w| w.some().unwrap(), "missing u8");
        mismatch::<Sparse>(
            |w| w.enum_index(2).unwrap(),
            "variant index 2 is out of range",
        );
        mismatch::<Packet>(
            |w| {
                w.i64(0).unwrap();
                w.u64(0).unwrap();
                w.u8(0).unwrap();
            },
            "does not match String expected at .text",
        );
        mismatch::<HashMap<u8, String>>(
            |w| {
                w.count(1).unwrap();
                w.u8(1).unwrap();
                w.u8(2).unwrap();
            },
            "[entry 0 value]",
        );
    }

    #[test]
    fn duplicate_keys_and_bad_arrays_are_rejected() {
        let mut writer = Writer::new();
        writer.count(2).unwrap();
        for _ in 0..2 {
            writer.u8(1).unwrap();
            writer.bool(true).unwrap();
        }
        let error = writer
            .validate(&TypeRef::new(&<HashMap<u8, bool>>::get_type()))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert_eq!(error.message, "duplicate map key");

        let mut writer = Writer::new();
        assert!(writer.array(DataKind::U32, &[0; 7], 2).is_err());
        assert!(writer.bytes().is_empty());
    }

    #[test]
    fn collections_split_into_items_and_entries() {
        let mut writer = Writer::new();
        writer.count(3).unwrap();
        for value in ["a", "", "ccc"] {
            writer.string(value).unwrap();
        }
        let whole = TypeRef::new(&<Vec<String>>::get_type());
        let items = writer.items(&whole).unwrap();
        let expected: Vec<_> = ["a", "", "ccc"]
            .iter()
            .map(|value| postcard::to_stdvec(value).unwrap())
            .collect();
        assert_eq!(items, expected);

        let mut writer = Writer::new();
        writer.count(2).unwrap();
        for (key, value) in [(1u16, 10u32), (300, 70000)] {
            writer.u16(key).unwrap();
            writer.u32(value).unwrap();
        }
        let whole = TypeRef::new(&<HashMap<u16, u32>>::get_type());
        let entries = writer.entries(&whole).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].0, postcard::to_stdvec(&300u16).unwrap());
        assert_eq!(entries[1].1, postcard::to_stdvec(&70000u32).unwrap());

        let mut writer = Writer::new();
        writer.count(3).unwrap();
        let whole = TypeRef::new(&<Vec<()>>::get_type());
        assert_eq!(writer.items(&whole).unwrap(), vec![Bytes::new(); 3]);
    }

    #[test]
    fn empty_writer_matches_zero_sized_types_only() {
        let writer = Writer::new();
        writer.validate(&TypeRef::new(&<()>::get_type())).unwrap();
        writer
            .validate(&TypeRef::new(&<((), [u8; 0])>::get_type()))
            .unwrap();
        assert!(writer.validate(&TypeRef::new(&u8::get_type())).is_err());
    }
}
