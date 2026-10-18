//! Compact type trees, the cursor that walks them value by value, and the
//! byte walker that checks and splits encoded values.
//!
//! The wire format is postcard: integers wider than one byte are varints
//! (zigzag for signed), floats are little-endian, `bool` and the `Option`
//! tag are one byte, strings and `Vec`/`HashMap` start with a varint length,
//! enums are their variant index, and structs, tuples, and arrays are their
//! fields in order. Structs, tuples, arrays, and `()` therefore have no header
//! of their own; the cursor steps into them.

use std::collections::HashSet;
use std::sync::Arc;

use serde::Deserialize;

use crate::serialization::deserialize_value;
use crate::typed::ObjectType;

use super::error::{Error, Result};
use super::kinds::{DataKind, ValueKind};

/// Largest element count accepted for collections of zero-sized elements,
/// such as `Vec<()>`, which have no encoded bytes to bound their length.
pub(crate) const MAX_ZERO_SIZED_ITEMS: u64 = 1 << 20;

pub(crate) type NodeId = u32;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Node {
    Num(DataKind),
    Bool,
    String,
    Empty,
    Enum {
        variants: u32,
    },
    Option(NodeId),
    Vec(NodeId),
    Map(NodeId, NodeId),
    List {
        element: NodeId,
        len: u32,
    },
    /// A tuple or struct; its children are `children[start..start + len]`.
    Fields {
        start: u32,
        len: u32,
    },
}

/// A type flattened into an arena of nodes.
#[derive(Debug)]
pub(crate) struct TypeTree {
    nodes: Vec<Node>,
    zero_sized: Vec<bool>,
    children: Vec<NodeId>,
    /// Field name of each entry in `children`, `None` for tuple elements.
    field_names: Vec<Option<String>>,
}

impl TypeTree {
    fn build(object_type: &ObjectType) -> (Self, NodeId) {
        let mut tree = Self {
            nodes: Vec::new(),
            zero_sized: Vec::new(),
            children: Vec::new(),
            field_names: Vec::new(),
        };
        let root = tree.add(object_type);
        (tree, root)
    }

    fn push(&mut self, node: Node, zero_sized: bool) -> NodeId {
        self.nodes.push(node);
        self.zero_sized.push(zero_sized);
        (self.nodes.len() - 1) as NodeId
    }

    fn add(&mut self, object_type: &ObjectType) -> NodeId {
        match object_type {
            ObjectType::U8 => self.push(Node::Num(DataKind::U8), false),
            ObjectType::U16 => self.push(Node::Num(DataKind::U16), false),
            ObjectType::U32 => self.push(Node::Num(DataKind::U32), false),
            ObjectType::U64 => self.push(Node::Num(DataKind::U64), false),
            ObjectType::I8 => self.push(Node::Num(DataKind::I8), false),
            ObjectType::I16 => self.push(Node::Num(DataKind::I16), false),
            ObjectType::I32 => self.push(Node::Num(DataKind::I32), false),
            ObjectType::I64 => self.push(Node::Num(DataKind::I64), false),
            ObjectType::F32 => self.push(Node::Num(DataKind::F32), false),
            ObjectType::F64 => self.push(Node::Num(DataKind::F64), false),
            ObjectType::Bool => self.push(Node::Bool, false),
            ObjectType::String => self.push(Node::String, false),
            ObjectType::Empty => self.push(Node::Empty, true),
            ObjectType::Enum(_, variants) => self.push(
                Node::Enum {
                    variants: variants.len() as u32,
                },
                false,
            ),
            ObjectType::Option(inner) => {
                let inner = self.add(inner);
                self.push(Node::Option(inner), false)
            }
            ObjectType::Vec(element) => {
                let element = self.add(element);
                self.push(Node::Vec(element), false)
            }
            ObjectType::Map(key, value) => {
                let key = self.add(key);
                let value = self.add(value);
                self.push(Node::Map(key, value), false)
            }
            ObjectType::List(len, element) => {
                let element = self.add(element);
                let zero_sized = *len == 0 || self.zero_sized[element as usize];
                self.push(Node::List { element, len: *len }, zero_sized)
            }
            ObjectType::Tuple(elements) => {
                let fields: Vec<_> = elements.iter().map(|e| (None, self.add(e))).collect();
                self.push_fields(fields)
            }
            ObjectType::Struct(_, fields) => {
                let fields: Vec<_> = fields
                    .iter()
                    .map(|(name, field)| (Some(name.clone()), self.add(field)))
                    .collect();
                self.push_fields(fields)
            }
        }
    }

    fn push_fields(&mut self, fields: Vec<(Option<String>, NodeId)>) -> NodeId {
        let start = self.children.len() as u32;
        let len = fields.len() as u32;
        let zero_sized = fields.iter().all(|(_, id)| self.zero_sized[*id as usize]);
        for (name, id) in fields {
            self.children.push(id);
            self.field_names.push(name);
        }
        self.push(Node::Fields { start, len }, zero_sized)
    }

    pub(crate) fn node(&self, id: NodeId) -> Node {
        self.nodes[id as usize]
    }

    pub(crate) fn is_zero_sized(&self, id: NodeId) -> bool {
        self.zero_sized[id as usize]
    }

    fn child(&self, start: u32, index: u32) -> NodeId {
        self.children[(start + index) as usize]
    }

    /// Kind of a node that carries a wire header or value.
    pub(crate) fn kind(&self, id: NodeId) -> ValueKind {
        match self.node(id) {
            Node::Num(kind) => ValueKind::from_data_kind(kind),
            Node::Bool => ValueKind::Bool,
            Node::String => ValueKind::String,
            Node::Option(_) => ValueKind::Option,
            Node::Vec(_) => ValueKind::Vec,
            Node::Map(..) => ValueKind::Map,
            Node::Enum { .. } => ValueKind::Enum,
            // The cursor steps into these and never yields them.
            Node::Empty | Node::List { .. } | Node::Fields { .. } => ValueKind::End,
        }
    }
}

/// A node of a shared [`TypeTree`]: the type of one state, collection item,
/// map key, or map value.
#[derive(Clone, Debug)]
pub(crate) struct TypeRef {
    pub(crate) tree: Arc<TypeTree>,
    pub(crate) node: NodeId,
}

impl TypeRef {
    /// Builds the tree of `object_type` and refers to its root.
    pub(crate) fn new(object_type: &ObjectType) -> Self {
        let (tree, node) = TypeTree::build(object_type);
        Self {
            tree: Arc::new(tree),
            node,
        }
    }

    /// Refers to another node of the same tree.
    pub(crate) fn at(&self, node: NodeId) -> Self {
        Self {
            tree: self.tree.clone(),
            node,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Frame {
    One(NodeId),
    Fields {
        start: u32,
        len: u32,
        next: u32,
    },
    Repeat {
        element: NodeId,
        remaining: u64,
        taken: u64,
    },
    Entries {
        key: NodeId,
        value: NodeId,
        remaining: u64,
        at_key: bool,
        taken: u64,
    },
}

/// Position in a value: the stack of values still expected.
#[derive(Clone, Debug, Default)]
pub(crate) struct Cursor {
    stack: Vec<Frame>,
}

impl Cursor {
    /// A cursor expecting one value of `node`.
    pub(crate) fn new(node: NodeId) -> Self {
        Self {
            stack: vec![Frame::One(node)],
        }
    }

    /// Advances to the next node carrying a wire header or value, stepping
    /// into structs, tuples, and arrays and over `()`. `None` means the value
    /// is complete.
    pub(crate) fn next_leaf(&mut self, tree: &TypeTree) -> Option<NodeId> {
        loop {
            let node = match self.stack.last_mut()? {
                Frame::One(node) => {
                    let node = *node;
                    self.stack.pop();
                    node
                }
                Frame::Fields { start, len, next } => {
                    if *next >= *len {
                        self.stack.pop();
                        continue;
                    }
                    *next += 1;
                    tree.child(*start, *next - 1)
                }
                Frame::Repeat {
                    element,
                    remaining,
                    taken,
                } => {
                    if *remaining == 0 {
                        self.stack.pop();
                        continue;
                    }
                    *remaining -= 1;
                    *taken += 1;
                    *element
                }
                Frame::Entries {
                    key,
                    value,
                    remaining,
                    at_key,
                    taken,
                } => {
                    if *remaining == 0 {
                        self.stack.pop();
                        continue;
                    }
                    if *at_key {
                        *at_key = false;
                        *taken += 1;
                        *key
                    } else {
                        *at_key = true;
                        *remaining -= 1;
                        *value
                    }
                }
            };

            // Zero-sized values have nothing to read or write.
            if tree.is_zero_sized(node) {
                continue;
            }
            match tree.node(node) {
                Node::Fields { start, len } => self.stack.push(Frame::Fields {
                    start,
                    len,
                    next: 0,
                }),
                Node::List { element, len } => self.stack.push(Frame::Repeat {
                    element,
                    remaining: u64::from(len),
                    taken: 0,
                }),
                _ => return Some(node),
            }
        }
    }

    /// Expects the inner value of a present `Option`.
    pub(crate) fn open_some(&mut self, inner: NodeId) {
        self.stack.push(Frame::One(inner));
    }

    /// Expects the `len` elements of a `Vec` whose header was consumed.
    pub(crate) fn open_vec(&mut self, tree: &TypeTree, element: NodeId, len: u64) -> Result<()> {
        if tree.is_zero_sized(element) {
            check_zero_sized_len(len)?;
            return Ok(());
        }
        self.stack.push(Frame::Repeat {
            element,
            remaining: len,
            taken: 0,
        });
        Ok(())
    }

    /// Expects the `len` entries of a `HashMap` whose header was consumed.
    pub(crate) fn open_map(
        &mut self,
        tree: &TypeTree,
        key: NodeId,
        value: NodeId,
        len: u64,
    ) -> Result<()> {
        if tree.is_zero_sized(key) && len > 1 {
            return Err(duplicate_key());
        }
        if tree.is_zero_sized(key) && tree.is_zero_sized(value) {
            return Ok(());
        }
        self.stack.push(Frame::Entries {
            key,
            value,
            remaining: len,
            at_key: true,
            taken: 0,
        });
        Ok(())
    }

    /// Location of the value most recently returned by [`Self::next_leaf`],
    /// such as `.points[3].x`, for diagnostics.
    pub(crate) fn path(&self, tree: &TypeTree) -> String {
        let mut path = String::new();
        for frame in &self.stack {
            match frame {
                Frame::One(_) => {}
                Frame::Fields { start, next, .. } if *next > 0 => {
                    let index = *start + *next - 1;
                    match &tree.field_names[index as usize] {
                        Some(name) => {
                            path.push('.');
                            path.push_str(name);
                        }
                        None => path.push_str(&format!(".{}", *next - 1)),
                    }
                }
                Frame::Fields { .. } => {}
                Frame::Repeat { taken, .. } if *taken > 0 => {
                    path.push_str(&format!("[{}]", taken - 1));
                }
                Frame::Repeat { .. } => {}
                Frame::Entries { taken, at_key, .. } if *taken > 0 => {
                    let part = if *at_key { "value" } else { "key" };
                    path.push_str(&format!("[entry {} {part}]", taken - 1));
                }
                Frame::Entries { .. } => {}
            }
        }
        if path.is_empty() {
            path.push_str("the value");
        }
        path
    }
}

fn check_zero_sized_len(len: u64) -> Result<()> {
    if len > MAX_ZERO_SIZED_ITEMS {
        return Err(Error::out_of_range(format!(
            "{len} zero-sized elements exceed the limit of {MAX_ZERO_SIZED_ITEMS}"
        )));
    }
    Ok(())
}

fn duplicate_key() -> Error {
    Error::invalid_argument("duplicate map key")
}

fn malformed(what: &str) -> Error {
    Error::invalid_argument(format!("malformed value data: invalid {what}"))
}

/// Decodes one postcard value of `T` at `pos`.
pub(crate) fn take<T>(bytes: &[u8], pos: usize, what: &str) -> Result<(T, usize)>
where
    T: for<'a> Deserialize<'a>,
{
    let rest = bytes.get(pos..).ok_or_else(|| malformed(what))?;
    let (value, used) = deserialize_value::<T>(rest).map_err(|_| malformed(what))?;
    Ok((value, pos + used))
}

/// Decodes a string at `pos`, returning its byte range.
pub(crate) fn take_str(bytes: &[u8], pos: usize) -> Result<(usize, usize)> {
    let rest = bytes.get(pos..).ok_or_else(|| malformed("string"))?;
    let (text, remaining) =
        postcard::take_from_bytes::<&str>(rest).map_err(|_| malformed("string"))?;
    let end = bytes.len() - remaining.len();
    Ok((end - text.len(), end))
}

/// Decodes a collection length and checks it against the remaining bytes.
pub(crate) fn take_len(
    tree: &TypeTree,
    element: NodeId,
    bytes: &[u8],
    pos: usize,
) -> Result<(u64, usize)> {
    let (len, pos) = take::<u64>(bytes, pos, "length")?;
    if tree.is_zero_sized(element) {
        check_zero_sized_len(len)?;
    } else if len > (bytes.len() - pos) as u64 {
        return Err(malformed("length"));
    }
    Ok((len, pos))
}

/// Decodes a number of `kind` at `pos` into its native-endian bytes.
pub(crate) fn take_num(kind: DataKind, bytes: &[u8], pos: usize) -> Result<([u8; 8], usize)> {
    let mut out = [0u8; 8];
    let what = kind.name();
    let pos = match kind {
        DataKind::U8 => put(&mut out, take::<u8>(bytes, pos, what)?, u8::to_ne_bytes),
        DataKind::U16 => put(&mut out, take::<u16>(bytes, pos, what)?, u16::to_ne_bytes),
        DataKind::U32 => put(&mut out, take::<u32>(bytes, pos, what)?, u32::to_ne_bytes),
        DataKind::U64 => put(&mut out, take::<u64>(bytes, pos, what)?, u64::to_ne_bytes),
        DataKind::I8 => put(&mut out, take::<i8>(bytes, pos, what)?, i8::to_ne_bytes),
        DataKind::I16 => put(&mut out, take::<i16>(bytes, pos, what)?, i16::to_ne_bytes),
        DataKind::I32 => put(&mut out, take::<i32>(bytes, pos, what)?, i32::to_ne_bytes),
        DataKind::I64 => put(&mut out, take::<i64>(bytes, pos, what)?, i64::to_ne_bytes),
        DataKind::F32 => put(&mut out, take::<f32>(bytes, pos, what)?, f32::to_ne_bytes),
        DataKind::F64 => put(&mut out, take::<f64>(bytes, pos, what)?, f64::to_ne_bytes),
    };
    Ok((out, pos))
}

fn put<T, const N: usize>(
    out: &mut [u8; 8],
    (value, pos): (T, usize),
    bytes: fn(T) -> [u8; N],
) -> usize {
    out[..N].copy_from_slice(&bytes(value));
    pos
}

/// Checks one encoded value of `node` at `pos` and returns the position after it.
///
/// Map keys must be unique within each map; writer output is canonical, so
/// byte equality is value equality.
pub(crate) fn walk(tree: &TypeTree, node: NodeId, bytes: &[u8], pos: usize) -> Result<usize> {
    if tree.is_zero_sized(node) {
        return Ok(pos);
    }
    match tree.node(node) {
        Node::Num(kind) => Ok(take_num(kind, bytes, pos)?.1),
        Node::Bool => Ok(take::<bool>(bytes, pos, "bool")?.1),
        Node::String => Ok(take_str(bytes, pos)?.1),
        Node::Empty => Ok(pos),
        Node::Enum { variants } => {
            let (index, pos) = take::<u32>(bytes, pos, "enum variant")?;
            if index >= variants {
                return Err(malformed("enum variant"));
            }
            Ok(pos)
        }
        Node::Option(inner) => match take::<u8>(bytes, pos, "option tag")? {
            (0, pos) => Ok(pos),
            (1, pos) => walk(tree, inner, bytes, pos),
            _ => Err(malformed("option tag")),
        },
        Node::Vec(element) => {
            let (len, mut pos) = take_len(tree, element, bytes, pos)?;
            if !tree.is_zero_sized(element) {
                for _ in 0..len {
                    pos = walk(tree, element, bytes, pos)?;
                }
            }
            Ok(pos)
        }
        Node::Map(key, value) => {
            let (len, mut pos) = take_len(tree, key, bytes, pos)?;
            if tree.is_zero_sized(key) {
                if len > 1 {
                    return Err(duplicate_key());
                }
                if tree.is_zero_sized(value) {
                    return Ok(pos);
                }
            }
            let mut keys = HashSet::new();
            for _ in 0..len {
                let start = pos;
                pos = walk(tree, key, bytes, pos)?;
                if !keys.insert(&bytes[start..pos]) {
                    return Err(duplicate_key());
                }
                pos = walk(tree, value, bytes, pos)?;
            }
            Ok(pos)
        }
        Node::List { element, len } => {
            let mut pos = pos;
            for _ in 0..len {
                pos = walk(tree, element, bytes, pos)?;
            }
            Ok(pos)
        }
        Node::Fields { start, len } => {
            let mut pos = pos;
            for index in 0..len {
                pos = walk(tree, tree.child(start, index), bytes, pos)?;
            }
            Ok(pos)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Typed;

    fn walk_all<T: Typed>(bytes: &[u8]) -> Result<usize> {
        let root = TypeRef::new(&T::get_type());
        walk(&root.tree, root.node, bytes, 0)
    }

    fn leaves<T: Typed>() -> Vec<ValueKind> {
        let root = TypeRef::new(&T::get_type());
        let mut cursor = Cursor::new(root.node);
        let mut kinds = Vec::new();
        while let Some(node) = cursor.next_leaf(&root.tree) {
            kinds.push(root.tree.kind(node));
        }
        kinds
    }

    #[test]
    fn cursor_steps_into_fixed_layouts_and_over_unit() {
        assert_eq!(
            leaves::<((), (u8, [i16; 2]), ())>(),
            [ValueKind::U8, ValueKind::I16, ValueKind::I16]
        );
        assert_eq!(leaves::<[(); 1000]>(), []);
        assert_eq!(leaves::<Option<u8>>(), [ValueKind::Option]);
        assert_eq!(leaves::<Vec<u8>>(), [ValueKind::Vec]);
    }

    #[test]
    fn walker_accepts_postcard_encodings() {
        let value: (
            Vec<Option<String>>,
            [u16; 2],
            std::collections::HashMap<u8, bool>,
        ) = (
            vec![Some("a".into()), None],
            [1, 70000u32 as u16],
            [(1, true), (2, false)].into(),
        );
        let bytes = postcard::to_stdvec(&value).unwrap();
        type T = (
            Vec<Option<String>>,
            [u16; 2],
            std::collections::HashMap<u8, bool>,
        );
        assert_eq!(walk_all::<T>(&bytes).unwrap(), bytes.len());
    }

    #[test]
    fn walker_rejects_malformed_data() {
        fn rejects<T: Typed>(bytes: &[u8], message: &str) {
            let error = walk_all::<T>(bytes).unwrap_err();
            assert!(error.message.contains(message), "{}", error.message);
        }
        rejects::<bool>(&[2], "invalid bool");
        rejects::<Option<u8>>(&[2, 0], "invalid option tag");
        rejects::<String>(&[2, 0xff, 0xfe], "invalid string");
        rejects::<String>(&[5, b'a'], "invalid string");
        rejects::<u32>(&[0x80], "invalid u32");
        rejects::<Vec<u8>>(&[0xff, 0xff, 0x03], "invalid length");
        rejects::<std::collections::HashMap<u8, u8>>(&[2, 1, 0, 1, 0], "duplicate map key");
        rejects::<std::collections::HashMap<(), u8>>(&[2, 0, 0], "duplicate map key");
        rejects::<Vec<()>>(&[0x81, 0x80, 0x80, 0x01], "zero-sized elements exceed");

        #[crate::typed]
        enum Two {
            A,
            B,
        }
        rejects::<Two>(&[2], "invalid enum variant");
        assert_eq!(walk_all::<Two>(&[1]).unwrap(), 1);
    }

    #[test]
    fn zero_sized_collections_do_not_iterate() {
        // 2^20 elements, encoded in three bytes.
        let bytes = [0x80, 0x80, 0x40];
        assert_eq!(walk_all::<Vec<()>>(&bytes).unwrap(), 3);
        let root = TypeRef::new(&<Vec<[(); 3]>>::get_type());
        let Node::Vec(element) = root.tree.node(root.node) else {
            unreachable!()
        };
        let mut cursor = Cursor::new(root.node);
        assert!(cursor.next_leaf(&root.tree).is_some());
        cursor
            .open_vec(&root.tree, element, MAX_ZERO_SIZED_ITEMS)
            .unwrap();
        assert!(cursor.next_leaf(&root.tree).is_none());
        assert!(
            cursor
                .open_vec(&root.tree, element, MAX_ZERO_SIZED_ITEMS + 1)
                .is_err()
        );
    }
}
