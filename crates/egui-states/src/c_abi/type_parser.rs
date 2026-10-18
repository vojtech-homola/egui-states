//! Parser for the Rust-syntax type strings of the C ABI.
//!
//! Type expressions use Rust spellings: primitives, `String`, `()`,
//! `Option<T>`, `Vec<T>`, `HashMap<K, V>`, `[T; N]`, tuples, and the names of
//! previously defined structs and enums. Definitions are written as
//! `struct Name { field: T, ... }` or `enum Name { A = 0, B, ... }`.

use crate::typed::ObjectType;

use super::error::{Error, ErrorCode, Result};

/// Type names that refer to built-in types and cannot name a definition.
const BUILTIN_NAMES: [&str; 15] = [
    "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64", "bool", "String", "Option",
    "Vec", "HashMap",
];

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Ident(String),
    Int(u128),
    Punct(char),
}

impl Token {
    fn describe(&self) -> String {
        match self {
            Token::Ident(name) => format!("`{name}`"),
            Token::Int(value) => format!("`{value}`"),
            Token::Punct(c) => format!("`{c}`"),
        }
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn tokenize(text: &str) -> Result<Vec<(Token, usize)>> {
    let mut tokens = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(offset, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if is_ident_start(c) {
            let mut end = offset;
            let mut raw = false;
            while let Some(&(index, c)) = chars.peek() {
                if is_ident_continue(c) {
                    end = index + c.len_utf8();
                    chars.next();
                } else if c == '#' && !raw && &text[offset..end] == "r" {
                    // `r#name` is a raw identifier; its spelling is kept verbatim.
                    raw = true;
                    end = index + 1;
                    chars.next();
                    match chars.peek() {
                        Some(&(_, next)) if is_ident_start(next) => {}
                        _ => {
                            return Err(syntax(format!(
                                "expected an identifier after `r#` at offset {}",
                                end
                            )));
                        }
                    }
                } else {
                    break;
                }
            }
            tokens.push((Token::Ident(text[offset..end].to_owned()), offset));
        } else if c.is_ascii_digit() {
            let mut end = offset;
            while let Some(&(index, c)) = chars.peek() {
                if c.is_ascii_alphanumeric() || c == '_' {
                    end = index + 1;
                    chars.next();
                } else {
                    break;
                }
            }
            let value = parse_int(&text[offset..end]).ok_or_else(|| {
                syntax(format!(
                    "invalid integer `{}` at offset {offset}",
                    &text[offset..end]
                ))
            })?;
            tokens.push((Token::Int(value), offset));
        } else if "<>,()[];:{}=-".contains(c) {
            tokens.push((Token::Punct(c), offset));
            chars.next();
        } else {
            return Err(syntax(format!(
                "unexpected character `{c}` at offset {offset}"
            )));
        }
    }
    Ok(tokens)
}

fn parse_int(literal: &str) -> Option<u128> {
    let digits = literal.replace('_', "");
    let (digits, radix) = match digits.get(..2) {
        Some("0x") | Some("0X") => (&digits[2..], 16),
        Some("0o") | Some("0O") => (&digits[2..], 8),
        Some("0b") | Some("0B") => (&digits[2..], 2),
        _ => (digits.as_str(), 10),
    };
    if digits.is_empty() {
        return None;
    }
    u128::from_str_radix(digits, radix).ok()
}

fn syntax(message: String) -> Error {
    Error::invalid_argument(message)
}

/// Parses a type expression, resolving names of user types with `resolve`.
pub(crate) fn parse_expression(
    text: &str,
    resolve: &dyn Fn(&str) -> Option<ObjectType>,
) -> Result<ObjectType> {
    let mut parser = Parser::new(text, resolve)?;
    let object_type = parser.parse_type()?;
    parser.expect_end()?;
    Ok(object_type)
}

/// Parses a `struct` or `enum` definition into a named [`ObjectType`].
pub(crate) fn parse_definition(
    text: &str,
    resolve: &dyn Fn(&str) -> Option<ObjectType>,
) -> Result<ObjectType> {
    let mut parser = Parser::new(text, resolve)?;
    let object_type = parser.parse_definition()?;
    parser.expect_end()?;
    Ok(object_type)
}

struct Parser<'a> {
    text_len: usize,
    tokens: Vec<(Token, usize)>,
    position: usize,
    resolve: &'a dyn Fn(&str) -> Option<ObjectType>,
}

impl<'a> Parser<'a> {
    fn new(text: &str, resolve: &'a dyn Fn(&str) -> Option<ObjectType>) -> Result<Self> {
        Ok(Self {
            text_len: text.len(),
            tokens: tokenize(text)?,
            position: 0,
            resolve,
        })
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position).map(|(token, _)| token)
    }

    fn offset(&self) -> usize {
        self.tokens
            .get(self.position)
            .map_or(self.text_len, |(_, offset)| *offset)
    }

    fn found(&self) -> String {
        self.peek()
            .map_or_else(|| "end of input".to_owned(), Token::describe)
    }

    fn error(&self, expected: &str) -> Error {
        syntax(format!(
            "expected {expected} at offset {}, found {}",
            self.offset(),
            self.found()
        ))
    }

    fn eat(&mut self, punct: char) -> bool {
        if self.peek() == Some(&Token::Punct(punct)) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, punct: char) -> Result<()> {
        if self.eat(punct) {
            Ok(())
        } else {
            Err(self.error(&format!("`{punct}`")))
        }
    }

    fn expect_end(&self) -> Result<()> {
        match self.peek() {
            None => Ok(()),
            Some(_) => Err(self.error("end of input")),
        }
    }

    fn ident(&mut self, expected: &str) -> Result<String> {
        match self.peek() {
            Some(Token::Ident(name)) => {
                let name = name.clone();
                self.position += 1;
                Ok(name)
            }
            _ => Err(self.error(expected)),
        }
    }

    fn generic_argument(&mut self) -> Result<ObjectType> {
        self.expect('<')?;
        let inner = self.parse_type()?;
        self.expect('>')?;
        Ok(inner)
    }

    fn parse_type(&mut self) -> Result<ObjectType> {
        let offset = self.offset();
        if self.eat('(') {
            return self.parse_parenthesized();
        }
        if self.eat('[') {
            let element = self.parse_type()?;
            self.expect(';')?;
            let size = match self.peek() {
                Some(&Token::Int(size)) => {
                    self.position += 1;
                    u32::try_from(size).map_err(|_| {
                        syntax(format!(
                            "array length at offset {offset} does not fit in u32"
                        ))
                    })?
                }
                _ => return Err(self.error("an array length")),
            };
            self.expect(']')?;
            return Ok(ObjectType::List(size, Box::new(element)));
        }

        let name = self.ident("a type")?;
        Ok(match name.as_str() {
            "u8" => ObjectType::U8,
            "u16" => ObjectType::U16,
            "u32" => ObjectType::U32,
            "u64" => ObjectType::U64,
            "i8" => ObjectType::I8,
            "i16" => ObjectType::I16,
            "i32" => ObjectType::I32,
            "i64" => ObjectType::I64,
            "f32" => ObjectType::F32,
            "f64" => ObjectType::F64,
            "bool" => ObjectType::Bool,
            "String" => ObjectType::String,
            "Option" => ObjectType::Option(Box::new(self.generic_argument()?)),
            "Vec" => ObjectType::Vec(Box::new(self.generic_argument()?)),
            "HashMap" => {
                self.expect('<')?;
                let key = self.parse_type()?;
                self.expect(',')?;
                let value = self.parse_type()?;
                self.expect('>')?;
                ObjectType::Map(Box::new(key), Box::new(value))
            }
            _ => (self.resolve)(&name).ok_or_else(|| {
                Error::new(
                    ErrorCode::NotFound,
                    format!("unknown type `{name}` at offset {offset}; define it first"),
                )
            })?,
        })
    }

    /// Parses after `(`: `()`, a parenthesized type, or a tuple.
    fn parse_parenthesized(&mut self) -> Result<ObjectType> {
        if self.eat(')') {
            return Ok(ObjectType::Empty);
        }
        let first = self.parse_type()?;
        if self.eat(')') {
            return Ok(first);
        }
        self.expect(',')?;
        let mut elements = vec![first];
        while !self.eat(')') {
            elements.push(self.parse_type()?);
            if !self.eat(',') {
                self.expect(')')?;
                break;
            }
        }
        Ok(ObjectType::Tuple(elements))
    }

    fn type_name(&mut self) -> Result<String> {
        let offset = self.offset();
        let name = self.ident("a type name")?;
        if BUILTIN_NAMES.contains(&name.as_str()) {
            return Err(syntax(format!(
                "`{name}` at offset {offset} is a built-in type and cannot be redefined"
            )));
        }
        Ok(name)
    }

    fn parse_definition(&mut self) -> Result<ObjectType> {
        match self.peek() {
            Some(Token::Ident(keyword)) if keyword == "struct" => {
                self.position += 1;
                self.parse_struct()
            }
            Some(Token::Ident(keyword)) if keyword == "enum" => {
                self.position += 1;
                self.parse_enum()
            }
            _ => Err(self.error("`struct` or `enum`")),
        }
    }

    fn parse_struct(&mut self) -> Result<ObjectType> {
        let name = self.type_name()?;
        let mut fields: Vec<(String, ObjectType)> = Vec::new();
        if self.eat(';') {
            return Ok(ObjectType::Struct(name, fields));
        }
        self.expect('{')?;
        while !self.eat('}') {
            let offset = self.offset();
            let field = self.ident("a field name")?;
            if fields.iter().any(|(existing, _)| *existing == field) {
                return Err(syntax(format!(
                    "duplicate field `{field}` at offset {offset} in struct `{name}`"
                )));
            }
            self.expect(':')?;
            let field_type = self.parse_type()?;
            fields.push((field, field_type));
            if !self.eat(',') {
                self.expect('}')?;
                break;
            }
        }
        Ok(ObjectType::Struct(name, fields))
    }

    fn parse_enum(&mut self) -> Result<ObjectType> {
        let name = self.type_name()?;
        self.expect('{')?;
        let mut variants: Vec<(String, i32)> = Vec::new();
        let mut next_value: Option<i64> = Some(0);
        while !self.eat('}') {
            let offset = self.offset();
            let variant = self.ident("a variant name")?;
            if variants.iter().any(|(existing, _)| *existing == variant) {
                return Err(syntax(format!(
                    "duplicate variant `{variant}` at offset {offset} in enum `{name}`"
                )));
            }
            let value = if self.eat('=') {
                self.discriminant()?
            } else {
                next_value.ok_or_else(|| {
                    syntax(format!(
                        "implicit discriminant of `{variant}` at offset {offset} overflows i32"
                    ))
                })?
            };
            let value = i32::try_from(value).map_err(|_| {
                syntax(format!(
                    "discriminant of `{variant}` at offset {offset} does not fit in i32"
                ))
            })?;
            if variants.iter().any(|(_, existing)| *existing == value) {
                return Err(syntax(format!(
                    "duplicate discriminant {value} of `{variant}` at offset {offset} in enum `{name}`"
                )));
            }
            next_value = i64::from(value)
                .checked_add(1)
                .filter(|next| i32::try_from(*next).is_ok());
            variants.push((variant, value));
            if !self.eat(',') {
                self.expect('}')?;
                break;
            }
        }
        Ok(ObjectType::Enum(name, variants))
    }

    fn discriminant(&mut self) -> Result<i64> {
        let offset = self.offset();
        let negative = self.eat('-');
        match self.peek() {
            Some(&Token::Int(value)) => {
                self.position += 1;
                let value = i128::try_from(value).ok();
                let value = value.map(|value| if negative { -value } else { value });
                value
                    .and_then(|value| i64::try_from(value).ok())
                    .ok_or_else(|| {
                        syntax(format!(
                            "discriminant at offset {offset} does not fit in i32"
                        ))
                    })
            }
            _ => Err(self.error("an integer discriminant")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_names(_: &str) -> Option<ObjectType> {
        None
    }

    fn expr(text: &str) -> Result<ObjectType> {
        parse_expression(text, &no_names)
    }

    fn definition(text: &str) -> Result<ObjectType> {
        parse_definition(text, &no_names)
    }

    #[test]
    fn expressions_use_rust_spellings() {
        use ObjectType::*;
        let cases = [
            ("u8", U8),
            (" i64 ", I64),
            ("String", String),
            ("bool", Bool),
            ("()", Empty),
            ("(u8)", U8),
            ("(u8,)", Tuple(vec![U8])),
            ("(u8, bool)", Tuple(vec![U8, Bool])),
            ("(u8, bool,)", Tuple(vec![U8, Bool])),
            ("((), i32)", Tuple(vec![Empty, I32])),
            ("[u16; 3]", List(3, Box::new(U16))),
            ("[u8;0x10]", List(16, Box::new(U8))),
            ("Vec<Vec<f32>>", Vec(Box::new(Vec(Box::new(F32))))),
            ("Option<()>", Option(Box::new(Empty))),
            (
                "HashMap<String, Option<(u8, [u16; 3])>>",
                Map(
                    Box::new(String),
                    Box::new(Option(Box::new(Tuple(vec![U8, List(3, Box::new(U16))])))),
                ),
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(expr(text).unwrap(), expected, "{text}");
        }
    }

    #[test]
    fn expression_errors_name_the_offset() {
        for (text, message) in [
            ("", "expected a type at offset 0, found end of input"),
            ("Vec<u8", "expected `>` at offset 6"),
            ("Vec u8", "expected `<` at offset 4"),
            ("[u8; -1]", "expected an array length at offset 5"),
            ("[u8; 4294967296]", "does not fit in u32"),
            ("u8 u8", "expected end of input at offset 3"),
            ("u8$", "unexpected character `$` at offset 2"),
            ("(u8 u8)", "expected `,` at offset 4"),
        ] {
            let error = expr(text).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidArgument, "{text}");
            assert!(error.message.contains(message), "{text}: {}", error.message);
        }

        let error = expr("Vec<Point>").unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
        assert!(error.message.contains("unknown type `Point`"));
    }

    #[test]
    fn names_resolve_through_the_callback() {
        let point = ObjectType::Struct("Point".into(), vec![("x".into(), ObjectType::I32)]);
        let resolve = |name: &str| (name == "Point").then(|| point.clone());
        assert_eq!(
            parse_expression("Option<Point>", &resolve).unwrap(),
            ObjectType::Option(Box::new(point.clone()))
        );
    }

    #[test]
    fn definitions_follow_the_typed_macro() {
        assert_eq!(
            definition("struct Point { x: i32, label: String, }").unwrap(),
            ObjectType::Struct(
                "Point".into(),
                vec![
                    ("x".into(), ObjectType::I32),
                    ("label".into(), ObjectType::String)
                ]
            )
        );
        assert_eq!(
            definition("struct Unit;").unwrap(),
            ObjectType::Struct("Unit".into(), vec![])
        );
        assert_eq!(
            definition("struct Unit {}").unwrap(),
            ObjectType::Struct("Unit".into(), vec![])
        );
        // Field and variant names may look like types; raw identifiers stay verbatim.
        assert_eq!(
            definition("struct Odd { u8: bool, r#type: u8 }").unwrap(),
            ObjectType::Struct(
                "Odd".into(),
                vec![
                    ("u8".into(), ObjectType::Bool),
                    ("r#type".into(), ObjectType::U8)
                ]
            )
        );
        assert_eq!(
            definition("enum Choice { A, B = 5, C, None = -2, D }").unwrap(),
            ObjectType::Enum(
                "Choice".into(),
                vec![
                    ("A".into(), 0),
                    ("B".into(), 5),
                    ("C".into(), 6),
                    ("None".into(), -2),
                    ("D".into(), -1)
                ]
            )
        );
        assert_eq!(
            definition("enum Extremes { Low = -2147483648, High = 2147483647 }").unwrap(),
            ObjectType::Enum(
                "Extremes".into(),
                vec![("Low".into(), i32::MIN), ("High".into(), i32::MAX)]
            )
        );
    }

    #[test]
    fn definition_errors() {
        for (text, message) in [
            ("struct u8 { x: i32 }", "is a built-in type"),
            ("enum Vec { A }", "is a built-in type"),
            ("struct P { x: i32, x: u8 }", "duplicate field `x`"),
            ("enum E { A, A }", "duplicate variant `A`"),
            ("enum E { A = 1, B = 1 }", "duplicate discriminant 1"),
            ("enum E { A = 0, B = 0 }", "duplicate discriminant 0"),
            ("enum E { A = 2147483648 }", "does not fit in i32"),
            (
                "enum E { A = 2147483647, B }",
                "implicit discriminant of `B`",
            ),
            ("enum E { A = x }", "expected an integer discriminant"),
            ("union U { x: u8 }", "expected `struct` or `enum`"),
            ("struct P { x: i32 } extra", "expected end of input"),
            ("struct P { r# }", "expected an identifier after `r#`"),
        ] {
            let error = definition(text).unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidArgument, "{text}");
            assert!(error.message.contains(message), "{text}: {}", error.message);
        }
    }
}
