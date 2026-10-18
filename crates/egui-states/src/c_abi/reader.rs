//! Type-checked decoder for values returned by the server.

use bytes::Bytes;

use super::error::{Error, Result};
use super::kinds::{DataKind, ValueKind};
use super::tree::{Cursor, Node, NodeId, TypeRef, take, take_len, take_num, take_str, walk};

/// Reusable, type-checked decoder for one value.
///
/// A reader is loaded by a getter or event with a value and its type. Values
/// are read in the order a [`Writer`](super::Writer) writes them; each read
/// must match the type of the next value, and a failed read leaves the
/// position unchanged.
#[derive(Clone, Debug, Default)]
pub struct Reader {
    bytes: Bytes,
    pos: usize,
    root: Option<TypeRef>,
    cursor: Cursor,
}

impl Reader {
    /// Creates an empty reader, which is at the end of its (missing) value.
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn load(&mut self, bytes: Bytes, root: &TypeRef) {
        self.bytes = bytes;
        self.pos = 0;
        self.cursor = Cursor::new(root.node);
        self.root = Some(root.clone());
    }

    /// Runs `read` and restores the position if it fails.
    fn step<R>(&mut self, read: impl FnOnce(&mut Self, &TypeRef) -> Result<R>) -> Result<R> {
        let Some(root) = self.root.clone() else {
            return Err(Error::type_mismatch("no more values: the reader is empty"));
        };
        let saved = (self.pos, self.cursor.clone());
        let result = read(self, &root);
        if result.is_err() {
            (self.pos, self.cursor) = saved;
        }
        result
    }

    /// Advances to the next value and checks that it is of `expected` kind.
    fn expect(&mut self, root: &TypeRef, expected: ValueKind) -> Result<NodeId> {
        let tree = &root.tree;
        let Some(node) = self.cursor.next_leaf(tree) else {
            return Err(Error::type_mismatch(format!(
                "no more values: cannot read {}",
                expected.name()
            )));
        };
        let actual = tree.kind(node);
        if actual != expected {
            return Err(Error::type_mismatch(format!(
                "cannot read {} at {}: the value is {}",
                expected.name(),
                self.cursor.path(tree),
                actual.name()
            )));
        }
        Ok(node)
    }

    fn num(&mut self, kind: DataKind) -> Result<[u8; 8]> {
        self.step(|reader, root| {
            reader.expect(root, ValueKind::from_data_kind(kind))?;
            let (value, pos) = take_num(kind, &reader.bytes, reader.pos)?;
            reader.pos = pos;
            Ok(value)
        })
    }

    /// Reads a `u8`.
    pub fn u8(&mut self) -> Result<u8> {
        Ok(u8::from_ne_bytes(first(self.num(DataKind::U8)?)))
    }

    /// Reads a `u16`.
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_ne_bytes(first(self.num(DataKind::U16)?)))
    }

    /// Reads a `u32`.
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_ne_bytes(first(self.num(DataKind::U32)?)))
    }

    /// Reads a `u64`.
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_ne_bytes(first(self.num(DataKind::U64)?)))
    }

    /// Reads an `i8`.
    pub fn i8(&mut self) -> Result<i8> {
        Ok(i8::from_ne_bytes(first(self.num(DataKind::I8)?)))
    }

    /// Reads an `i16`.
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_ne_bytes(first(self.num(DataKind::I16)?)))
    }

    /// Reads an `i32`.
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_ne_bytes(first(self.num(DataKind::I32)?)))
    }

    /// Reads an `i64`.
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_ne_bytes(first(self.num(DataKind::I64)?)))
    }

    /// Reads an `f32`.
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_ne_bytes(first(self.num(DataKind::F32)?)))
    }

    /// Reads an `f64`.
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_ne_bytes(first(self.num(DataKind::F64)?)))
    }

    /// Reads a `bool`.
    pub fn bool(&mut self) -> Result<bool> {
        self.step(|reader, root| {
            reader.expect(root, ValueKind::Bool)?;
            let (value, pos) = take::<bool>(&reader.bytes, reader.pos, "bool")?;
            reader.pos = pos;
            Ok(value)
        })
    }

    /// Reads a `String`, borrowed until the reader is reloaded or dropped.
    pub fn string(&mut self) -> Result<&str> {
        let (start, end) = self.step(|reader, root| {
            reader.expect(root, ValueKind::String)?;
            let range = take_str(&reader.bytes, reader.pos)?;
            reader.pos = range.1;
            Ok(range)
        })?;
        std::str::from_utf8(&self.bytes[start..end])
            .map_err(|_| Error::internal("validated string is not UTF-8"))
    }

    /// Reads whether an `Option` is present; its value follows when it is.
    pub fn option(&mut self) -> Result<bool> {
        self.step(|reader, root| {
            let node = reader.expect(root, ValueKind::Option)?;
            let Node::Option(inner) = root.tree.node(node) else {
                return Err(Error::internal("option node expected"));
            };
            match take::<u8>(&reader.bytes, reader.pos, "option tag")? {
                (0, pos) => {
                    reader.pos = pos;
                    Ok(false)
                }
                (1, pos) => {
                    reader.pos = pos;
                    if !root.tree.is_zero_sized(inner) {
                        reader.cursor.open_some(inner);
                    }
                    Ok(true)
                }
                _ => Err(Error::invalid_argument(
                    "malformed value data: invalid option tag",
                )),
            }
        })
    }

    /// Reads the element count of a `Vec` or the entry count of a `HashMap`;
    /// the elements or key/value pairs follow.
    pub fn count(&mut self) -> Result<u64> {
        self.step(|reader, root| {
            let tree = &root.tree;
            let Some(node) = reader.cursor.clone().next_leaf(tree) else {
                return Err(Error::type_mismatch("no more values: cannot read a length"));
            };
            let kind = tree.kind(node);
            let expected = if kind == ValueKind::Map {
                ValueKind::Map
            } else {
                ValueKind::Vec
            };
            reader.expect(root, expected)?;
            match tree.node(node) {
                Node::Vec(element) => {
                    let (len, pos) = take_len(tree, element, &reader.bytes, reader.pos)?;
                    reader.cursor.open_vec(tree, element, len)?;
                    reader.pos = pos;
                    Ok(len)
                }
                Node::Map(key, value) => {
                    let (len, pos) = take_len(tree, key, &reader.bytes, reader.pos)?;
                    reader.cursor.open_map(tree, key, value, len)?;
                    reader.pos = pos;
                    Ok(len)
                }
                _ => Err(Error::internal("collection node expected")),
            }
        })
    }

    /// Reads an enum as its variant index in declaration order.
    pub fn enum_index(&mut self) -> Result<u32> {
        self.step(|reader, root| {
            let node = reader.expect(root, ValueKind::Enum)?;
            let Node::Enum { variants } = root.tree.node(node) else {
                return Err(Error::internal("enum node expected"));
            };
            let (index, pos) = take::<u32>(&reader.bytes, reader.pos, "enum variant")?;
            if index >= variants {
                return Err(Error::invalid_argument(
                    "malformed value data: invalid enum variant",
                ));
            }
            reader.pos = pos;
            Ok(index)
        })
    }

    /// Reads `count` numbers of `kind` into native-endian `out` bytes, as if
    /// each was read on its own.
    pub fn array(&mut self, kind: DataKind, out: &mut [u8], count: usize) -> Result<()> {
        if count.checked_mul(kind.size()) != Some(out.len()) {
            return Err(Error::invalid_argument(format!(
                "{} bytes do not hold {count} {} values",
                out.len(),
                kind.name()
            )));
        }
        self.step(|reader, root| {
            for chunk in out.chunks_exact_mut(kind.size()) {
                reader.expect(root, ValueKind::from_data_kind(kind))?;
                let (value, pos) = take_num(kind, &reader.bytes, reader.pos)?;
                reader.pos = pos;
                chunk.copy_from_slice(&value[..kind.size()]);
            }
            Ok(())
        })
    }

    /// Returns the kind of the next value, or [`ValueKind::End`].
    pub fn peek(&self) -> ValueKind {
        let Some(root) = &self.root else {
            return ValueKind::End;
        };
        match self.cursor.clone().next_leaf(&root.tree) {
            Some(node) => root.tree.kind(node),
            None => ValueKind::End,
        }
    }

    /// Skips the next value, including everything it contains.
    pub fn skip(&mut self) -> Result<()> {
        self.step(|reader, root| {
            let Some(node) = reader.cursor.next_leaf(&root.tree) else {
                return Err(Error::type_mismatch("no more values: nothing to skip"));
            };
            reader.pos = walk(&root.tree, node, &reader.bytes, reader.pos)?;
            Ok(())
        })
    }

    /// Returns whether every value has been read.
    pub fn at_end(&self) -> bool {
        self.peek() == ValueKind::End
    }
}

fn first<const N: usize>(bytes: [u8; 8]) -> [u8; N] {
    bytes[..N].try_into().expect("N is at most 8")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::Typed;
    use crate::c_abi::ErrorCode;

    fn reader_for<T: Typed + serde::Serialize>(value: &T) -> Reader {
        let mut reader = Reader::new();
        reader.load(
            Bytes::from(postcard::to_stdvec(value).unwrap()),
            &TypeRef::new(&T::get_type()),
        );
        reader
    }

    #[crate::typed]
    struct Item {
        id: u16,
        tags: Vec<String>,
    }

    #[test]
    fn reads_serde_encodings() {
        // Tuples have no wire header, so the nesting does not show in the reads.
        type T = (
            (
                i64,
                u64,
                Option<(f32, f64)>,
                [i8; 2],
                HashMap<u8, Option<Item>>,
            ),
            ((), String, bool, i16, u32, i32),
        );
        let value: T = (
            (
                i64::MIN,
                u64::MAX,
                Some((-0.5, f64::INFINITY)),
                [-128, 127],
                [(
                    7,
                    Some(Item {
                        id: 65535,
                        tags: vec!["a".into(), "中".into()],
                    }),
                )]
                .into(),
            ),
            ((), "tail".into(), true, -300, 70000, i32::MIN),
        );
        let mut reader = reader_for(&value);
        assert_eq!(reader.i64().unwrap(), i64::MIN);
        assert_eq!(reader.u64().unwrap(), u64::MAX);
        assert!(reader.option().unwrap());
        assert_eq!(reader.f32().unwrap(), -0.5);
        assert_eq!(reader.f64().unwrap(), f64::INFINITY);
        let mut pair = [0u8; 2];
        reader.array(DataKind::I8, &mut pair, 2).unwrap();
        assert_eq!(pair.map(|b| b as i8), [-128, 127]);
        assert_eq!(reader.peek(), ValueKind::Map);
        assert_eq!(reader.count().unwrap(), 1);
        assert_eq!(reader.u8().unwrap(), 7);
        assert!(reader.option().unwrap());
        assert_eq!(reader.u16().unwrap(), 65535);
        assert_eq!(reader.count().unwrap(), 2);
        assert_eq!(reader.string().unwrap(), "a");
        assert_eq!(reader.string().unwrap(), "中");
        assert_eq!(reader.string().unwrap(), "tail");
        assert!(reader.bool().unwrap());
        assert_eq!(reader.i16().unwrap(), -300);
        assert_eq!(reader.u32().unwrap(), 70000);
        assert_eq!(reader.i32().unwrap(), i32::MIN);
        assert!(reader.at_end());
    }

    #[test]
    fn failed_reads_do_not_advance() {
        let mut reader = reader_for(&(5u32, String::from("x")));
        let error = reader.i32().unwrap_err();
        assert_eq!(error.code, ErrorCode::TypeMismatch);
        assert_eq!(error.message, "cannot read i32 at .0: the value is u32");
        assert_eq!(reader.u32().unwrap(), 5);
        assert!(reader.u8().is_err());
        assert_eq!(reader.string().unwrap(), "x");
        let error = reader.u8().unwrap_err();
        assert!(error.message.starts_with("no more values"));

        let mut empty = Reader::new();
        assert!(empty.at_end());
        assert!(empty.u8().is_err());
    }

    #[test]
    fn skip_and_peek() {
        let mut reader = reader_for(&(vec![Some(1u8), None], Some(String::from("x")), 3i8));
        assert_eq!(reader.peek(), ValueKind::Vec);
        reader.skip().unwrap();
        assert_eq!(reader.peek(), ValueKind::Option);
        reader.skip().unwrap();
        assert_eq!(reader.i8().unwrap(), 3);
        assert!(reader.skip().is_err());
    }

    #[test]
    fn malformed_bytes_are_errors_not_panics() {
        let encoded = postcard::to_stdvec(&(String::from("hello"), vec![1u64, 2, 3])).unwrap();
        let root = TypeRef::new(&<(String, Vec<u64>)>::get_type());
        for len in 0..encoded.len() {
            let mut reader = Reader::new();
            reader.load(Bytes::copy_from_slice(&encoded[..len]), &root);
            let mut read_all = || -> Result<()> {
                reader.string()?;
                for _ in 0..reader.count()? {
                    reader.u64()?;
                }
                Ok(())
            };
            let error = read_all().unwrap_err();
            assert_eq!(
                error.code,
                ErrorCode::InvalidArgument,
                "{len}: {}",
                error.message
            );
        }
    }
}
