//! Named type definitions of one server.

use std::collections::HashMap;

use crate::typed::ObjectType;

use super::error::{Error, ErrorCode, Result};
use super::type_parser;

/// Struct and enum definitions registered on a server, by name.
#[derive(Clone, Default)]
pub(crate) struct TypeRegistry {
    named: HashMap<String, ObjectType>,
}

impl TypeRegistry {
    /// Registers one `struct` or `enum` definition.
    ///
    /// Names it uses must already be defined. Repeating an identical
    /// definition succeeds; a different definition of the same name fails.
    pub(crate) fn define(&mut self, text: &str) -> Result<()> {
        let definition = type_parser::parse_definition(text, &|name| self.lookup(name))?;
        let name = match &definition {
            ObjectType::Struct(name, _) | ObjectType::Enum(name, _) => name.clone(),
            _ => return Err(Error::internal("definition did not produce a named type")),
        };
        match self.named.get(&name) {
            Some(existing) if *existing == definition => Ok(()),
            Some(existing) => Err(Error::new(
                ErrorCode::AlreadyExists,
                format!(
                    "type `{name}` is already defined as `{}`",
                    crate::typed::type_definition(existing).unwrap_or_default()
                ),
            )),
            None => {
                self.named.insert(name, definition);
                Ok(())
            }
        }
    }

    /// Parses a type expression, resolving defined names.
    pub(crate) fn parse(&self, text: &str) -> Result<ObjectType> {
        type_parser::parse_expression(text, &|name| self.lookup(name))
    }

    fn lookup(&self, name: &str) -> Option<ObjectType> {
        self.named.get(name).cloned()
    }
}

/// Protocol hash of a map state with the given key and value types.
pub(crate) fn map_hash(key: &ObjectType, value: &ObjectType) -> u32 {
    value.get_hash_from(key.get_hash())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::Typed;

    #[crate::typed]
    struct Inner {
        flag: bool,
        level: u16,
    }

    #[crate::typed]
    enum Sparse {
        Low = -7,
        Mid = 3,
        High = 201,
    }

    #[crate::typed]
    struct Outer {
        inner: Inner,
        choice: Option<Sparse>,
        values: Vec<(u8, [u16; 3])>,
        names: HashMap<String, Inner>,
        unit: (),
    }

    #[crate::typed]
    struct Empty {}

    fn registry() -> TypeRegistry {
        let mut registry = TypeRegistry::default();
        registry
            .define("struct Inner { flag: bool, level: u16 }")
            .unwrap();
        registry
            .define("enum Sparse { Low = -7, Mid = 3, High = 201 }")
            .unwrap();
        registry
            .define(
                "struct Outer { inner: Inner, choice: Option<Sparse>, \
                 values: Vec<(u8, [u16; 3])>, names: HashMap<String, Inner>, unit: () }",
            )
            .unwrap();
        registry.define("struct Empty {}").unwrap();
        registry
    }

    fn assert_same<T: Typed>(registry: &TypeRegistry, text: &str) {
        let parsed = registry.parse(text).unwrap();
        assert_eq!(parsed, T::get_type(), "{text}");
        assert_eq!(parsed.get_hash(), T::get_type().get_hash(), "{text}");
    }

    /// Clients compare these hashes on every message, so a type string must
    /// describe exactly the Rust type it is written after.
    #[test]
    fn parsed_types_hash_like_their_rust_types() {
        let registry = registry();
        assert_same::<u8>(&registry, "u8");
        assert_same::<f64>(&registry, "f64");
        assert_same::<String>(&registry, "String");
        assert_same::<()>(&registry, "()");
        assert_same::<Option<()>>(&registry, "Option<()>");
        assert_same::<((), i32)>(&registry, "((), i32)");
        assert_same::<(u8,)>(&registry, "(u8,)");
        assert_same::<[u16; 3]>(&registry, "[u16; 3]");
        assert_same::<HashMap<String, Vec<Option<(u8, [u16; 3])>>>>(
            &registry,
            "HashMap<String, Vec<Option<(u8, [u16; 3])>>>",
        );
        assert_same::<Inner>(&registry, "Inner");
        assert_same::<Sparse>(&registry, "Sparse");
        assert_same::<Outer>(&registry, "Outer");
        assert_same::<Empty>(&registry, "Empty");
        assert_same::<Vec<HashMap<Sparse, Outer>>>(&registry, "Vec<HashMap<Sparse, Outer>>");
    }

    #[test]
    fn printed_types_parse_back_unchanged() {
        let registry = registry();
        for object_type in [
            Outer::get_type(),
            <HashMap<String, Vec<Option<(u8, [u16; 3])>>>>::get_type(),
            <((), (i8,), [Sparse; 2])>::get_type(),
        ] {
            let text = crate::typed::type_expression(&object_type);
            assert_eq!(registry.parse(&text).unwrap(), object_type, "{text}");
        }
        // Definitions are printed in dependency order, so each one only uses
        // names defined before it.
        let mut fresh = TypeRegistry::default();
        for definition in [
            Inner::get_type(),
            Sparse::get_type(),
            Outer::get_type(),
            Empty::get_type(),
        ] {
            let text = crate::typed::type_definition(&definition).unwrap();
            fresh.define(&text).unwrap();
            let (ObjectType::Struct(name, _) | ObjectType::Enum(name, _)) = &definition else {
                unreachable!()
            };
            assert_eq!(fresh.parse(name).unwrap(), definition, "{text}");
        }
    }

    #[test]
    fn redefinition_must_be_identical() {
        let mut registry = registry();
        registry
            .define("struct Inner { flag: bool, level: u16 }")
            .unwrap();
        // Field names do not change the hash but still make it a different definition.
        let error = registry
            .define("struct Inner { enabled: bool, level: u16 }")
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AlreadyExists);
        assert!(
            error
                .message
                .contains("struct Inner { flag: bool, level: u16 }")
        );

        let error = registry.define("struct Uses { x: Missing }").unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
    }
}
