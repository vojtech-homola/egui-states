//! Procedural macros for the `egui_states` crate.

use proc_macro::TokenStream;
use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::{
    Attribute, DeriveInput, Meta, Path, Token, parse::Parser, parse_macro_input,
    punctuated::Punctuated,
};

mod objects;
mod states;

/// The expression inside any invisible groups. A fragment forwarded through
/// `macro_rules!`, such as `$value:expr`, arrives wrapped in one.
fn ungroup(mut expr: &syn::Expr) -> &syn::Expr {
    while let syn::Expr::Group(group) = expr {
        expr = &group.expr;
    }
    expr
}

/// Implements `egui_states::Typed` and derives Serde serialization through
/// the `egui_states` Serde re-export.
///
/// Use `rust_derive(...)` to add derives to the corresponding generated Rust
/// server type. Derive paths are emitted as written and must be available in the
/// crate compiling the generated bindings. They are not added to the source
/// type, and prerequisite derives are not inferred.
///
/// The attribute supports non-generic unit structs or structs with named
/// fields, and non-generic enums with at least one variant. A fieldless enum
/// may have discriminants that fit in `i32`. An enum with data may mix unit
/// variants, variants with named fields and tuple variants, but has no
/// explicit discriminants and no empty `Variant {}` or `Variant()` variants.
/// Unions, tuple structs and generic items are not supported. Every field type
/// must implement `egui_states::Typed`.
///
/// A type cannot contain itself. A field that names its own type, such as
/// `children: Vec<Node>` in `Node`, is a compile error. A type that reaches
/// itself through another type or a type alias compiles, and describing it
/// panics with the name of the type.
///
/// The type description is the same in every build, so the encoding must be
/// too. Serde attributes that change the encoding, such as `skip` or
/// `flatten`, are compile errors, and so is any `serde` attribute inside
/// `cfg_attr`. A `#[cfg(...)]` on a field or a variant is a compile error as
/// well, also inside `cfg_attr`: a build without the member would still
/// describe it, while its values are encoded without it. Put the condition on
/// the whole type, or keep the member in every build, as an `Option` for
/// example. Other uses of `cfg_attr`, such as for a derive or documentation,
/// are accepted.
#[proc_macro_attribute]
pub fn typed(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = TokenStream2::from(args);
    let rust_derives = match parse_typed_args(args) {
        Ok(rust_derives) => rust_derives,
        Err(error) => return error.into_compile_error().into(),
    };

    let item_input = input.clone();
    let mut item = parse_macro_input!(item_input as DeriveInput);
    let typed_impl = TokenStream2::from(objects::impl_typed(input, &rust_derives));

    match add_serde_attributes(&mut item) {
        Ok(()) => quote! {
            #item
            #typed_impl
        }
        .into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn parse_typed_args(args: TokenStream2) -> syn::Result<Vec<Path>> {
    if args.is_empty() {
        return Ok(Vec::new());
    }

    let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
    let metas = parser.parse2(args)?;
    let mut rust_derives = None;

    for meta in metas {
        let Meta::List(list) = &meta else {
            return Err(syn::Error::new_spanned(meta, "expected `rust_derive(...)`"));
        };
        if !list.path.is_ident("rust_derive") {
            return Err(syn::Error::new_spanned(
                &list.path,
                "unknown `typed` option; expected `rust_derive(...)`",
            ));
        }
        if rust_derives.is_some() {
            return Err(syn::Error::new_spanned(
                list,
                "duplicate `rust_derive(...)` option",
            ));
        }

        let derives = Punctuated::<Path, Token![,]>::parse_terminated
            .parse2(list.tokens.clone())?
            .into_iter()
            .collect::<Vec<_>>();
        if derives.is_empty() {
            return Err(syn::Error::new_spanned(
                list,
                "`rust_derive(...)` requires at least one derive path",
            ));
        }
        rust_derives = Some(derives);
    }

    Ok(rust_derives.unwrap_or_default())
}

/// Derives `egui_states::InitialValue` for a struct or enum.
///
/// This supports the same item shapes as [`typed`]. Every struct field must
/// implement `egui_states::InitialValue`.
#[proc_macro_derive(InitialValue)]
pub fn initial_value(input: TokenStream) -> TokenStream {
    objects::impl_initial_value(input)
}

/// Derives `egui_states::Atomic`, so the type can be stored in a
/// `ValueAtomic` and a `StaticAtomic`.
///
/// The input must be a non-generic fieldless enum whose discriminants fit in
/// `i32`; the generated implementation stores the discriminant atomically. An
/// enum with data that fits in 64 bits can implement `egui_states::Atomic` by
/// hand instead.
#[proc_macro_derive(Atomic)]
pub fn atomic(input: TokenStream) -> TokenStream {
    objects::impl_atomic(input)
}

/// Derives `egui_states::State` for a struct whose fields are state handles.
///
/// Each field is registered under its Rust field name. Nested structs that
/// also derive `State` become nested state groups. The struct must be
/// non-generic, contain at least one named field, and use path-based field
/// types. Recognized state-handle names are initialized through the matching
/// `StatesCreator` method; other field types are treated as nested states.
///
/// A field takes options in a `#[state(...)]` attribute:
///
/// - `low_priority`: the server sends the field's transfers in its
///   low-priority lane, as `StatesCreator::low_priority` describes. It applies
///   to `Image`, `ImageMulti`, `Data`, `DataTake`, `DataMulti` and
///   `DataMultiTake`, and on a nested state to every such handle inside it, at
///   any depth.
/// - `queue`: server callbacks receive every change of the field in arrival
///   order, as `StatesCreator::queue` describes, instead of only the latest
///   one. It applies to `Value`, `ValueAtomic` and `Signal`. On a
///   `ValueTake`, the client keeps up to `VALUE_TAKE_QUEUE` (100) untaken
///   values in arrival order instead of only the latest.
/// - `queue = N`: a `ValueTake` keeps up to `N` untaken values, at least 1.
///   `N` is an integer literal, plain or with the `usize` suffix.
/// - `init = <expr>`: the field starts from this value instead of
///   `Default::default()`, on the client and in the generated server bindings.
///   It applies to `Value`, `ValueAtomic`, `Static` and `StaticAtomic`. The
///   expression is any Rust expression of the field's value type; a string
///   literal on its own is converted with `Into`, so it also initializes a
///   `String`. The expression is evaluated both when the server bindings are
///   generated and when the client starts, so it must give the same value each
///   time: a value that depends on the environment, the clock, or the target
///   platform would differ between the server and the client.
///
/// Options can be combined in one attribute, `#[state(a, b)]`.
///
/// ```ignore
/// #[derive(egui_states::State)]
/// struct Views {
///     result: Image,
///     #[state(low_priority)]
///     camera: Image,
///     #[state(queue)]
///     clicks: Signal<u32>,
///     #[state(queue = 20)]
///     events: ValueTake<String>,
///     #[state(init = 0.5)]
///     gain: Value<f32>,
///     #[state(init = "ready")]
///     status: Static<String>,
/// }
/// ```
#[proc_macro_derive(State, attributes(state))]
pub fn state(input: TokenStream) -> TokenStream {
    states::impl_state(input)
}

/// Serde attributes that leave the postcard encoding unchanged. Every other
/// attribute (`skip`, `flatten`, `untagged`, `with`, ...) would make `Typed`
/// describe a different wire format than the one Serde produces.
/// `transparent` qualifies because Serde only accepts it on a struct with a
/// single field, which postcard encodes exactly like that field.
const WIRE_NEUTRAL_SERDE_ATTRIBUTES: &[&str] = &[
    "transparent",
    "rename",
    "rename_all",
    "alias",
    "default",
    "deny_unknown_fields",
    "crate",
    "bound",
    "expecting",
];

/// The error for a `cfg` on a field or a variant: the macro sees the member
/// whatever the condition, so `Typed` would describe a member that a build
/// may not have.
fn cfg_member_error(tokens: impl quote::ToTokens) -> syn::Error {
    syn::Error::new_spanned(
        tokens,
        "`#[cfg]` on a field or a variant is not supported by `#[typed]`: `Typed` describes one wire encoding for every build; put the condition on the whole type, or keep the member in every build",
    )
}

/// Rejects a `serde` attribute in the body of a `cfg_attr`, also in a
/// `cfg_attr` nested in it. Such an attribute would change the encoding only
/// when the condition holds, while `Typed` describes one encoding. On a
/// `member`, a field or a variant, a `cfg` in the body is rejected too.
fn check_cfg_attr(tokens: TokenStream2, member: bool) -> syn::Result<()> {
    // `condition, attribute, attribute, ...`: the items are split by the
    // commas outside of any brackets, and an attribute starts with its name.
    let mut item = 0;
    let mut item_start = true;
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        match &token {
            TokenTree::Punct(punct) if punct.as_char() == ',' => {
                item += 1;
                item_start = true;
                continue;
            }
            TokenTree::Ident(name) if item_start && item > 0 => {
                if name == "serde" {
                    return Err(syn::Error::new_spanned(
                        name,
                        "a `serde` attribute inside `cfg_attr` is not supported by `#[typed]`: `Typed` describes one wire encoding, whatever the condition",
                    ));
                }
                if member && name == "cfg" {
                    return Err(cfg_member_error(name));
                }
                if name == "cfg_attr"
                    && let Some(TokenTree::Group(group)) = tokens.peek()
                {
                    check_cfg_attr(group.stream(), member)?;
                }
            }
            _ => {}
        }
        item_start = false;
    }
    Ok(())
}

/// Checks the attributes of a typed item.
fn check_serde_attributes(attrs: &[Attribute]) -> syn::Result<()> {
    check_attributes(attrs, false)
}

/// Checks the attributes of a field or a variant of a typed item, where a
/// `cfg` is rejected as well.
fn check_member_attributes(attrs: &[Attribute]) -> syn::Result<()> {
    check_attributes(attrs, true)
}

fn check_attributes(attrs: &[Attribute], member: bool) -> syn::Result<()> {
    let parser = Punctuated::<Meta, Token![,]>::parse_terminated;

    if member && let Some(attr) = attrs.iter().find(|attr| attr.path().is_ident("cfg")) {
        return Err(cfg_member_error(attr));
    }

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("cfg_attr")) {
        check_cfg_attr(attr.meta.require_list()?.tokens.clone(), member)?;
    }

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        for meta in parser.parse2(attr.meta.require_list()?.tokens.clone())? {
            let path = meta.path();
            if !WIRE_NEUTRAL_SERDE_ATTRIBUTES
                .iter()
                .any(|allowed| path.is_ident(allowed))
            {
                let name = quote!(#path).to_string();
                return Err(syn::Error::new_spanned(
                    path,
                    format!(
                        "serde attribute `{name}` changes the wire encoding described by `Typed` and is not supported by `#[typed]`"
                    ),
                ));
            }
        }
    }

    Ok(())
}

fn add_serde_attributes(item: &mut DeriveInput) -> syn::Result<()> {
    check_serde_attributes(&item.attrs)?;
    match &item.data {
        syn::Data::Struct(data) => {
            for field in &data.fields {
                check_member_attributes(&field.attrs)?;
            }
        }
        syn::Data::Enum(data) => {
            for variant in &data.variants {
                check_member_attributes(&variant.attrs)?;
                for field in &variant.fields {
                    check_member_attributes(&field.attrs)?;
                }
            }
        }
        syn::Data::Union(_) => {}
    }

    let (has_serialize, has_deserialize) = serde_derives(&item.attrs)?;
    let mut missing_derives = Vec::new();

    if !has_serialize {
        missing_derives.push(quote!(egui_states::serde::Serialize));
    }
    if !has_deserialize {
        missing_derives.push(quote!(egui_states::serde::Deserialize));
    }

    if !missing_derives.is_empty() {
        let serde_attribute_index = item
            .attrs
            .iter()
            .position(|attr| attr.path().is_ident("serde"))
            .unwrap_or(item.attrs.len());
        item.attrs.insert(
            serde_attribute_index,
            syn::parse_quote!(#[derive(#(#missing_derives),*)]),
        );
    }

    if !has_serde_crate_override(&item.attrs)? {
        item.attrs
            .push(syn::parse_quote!(#[serde(crate = "egui_states::serde")]));
    }

    Ok(())
}

fn serde_derives(attrs: &[Attribute]) -> syn::Result<(bool, bool)> {
    let parser = Punctuated::<Path, Token![,]>::parse_terminated;
    let mut has_serialize = false;
    let mut has_deserialize = false;

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("derive")) {
        for path in parser.parse2(attr.meta.require_list()?.tokens.clone())? {
            let Some(derive) = path.segments.last() else {
                continue;
            };
            has_serialize |= derive.ident == "Serialize";
            has_deserialize |= derive.ident == "Deserialize";
        }
    }

    Ok((has_serialize, has_deserialize))
}

fn has_serde_crate_override(attrs: &[Attribute]) -> syn::Result<bool> {
    let parser = Punctuated::<Meta, Token![,]>::parse_terminated;

    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        let metas = parser.parse2(attr.meta.require_list()?.tokens.clone())?;
        if metas.iter().any(|meta| match meta {
            Meta::NameValue(value) => value.path.is_ident("crate"),
            Meta::Path(_) | Meta::List(_) => false,
        }) {
            return Ok(true);
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use quote::{ToTokens, quote};

    use super::{add_serde_attributes, check_serde_attributes, parse_typed_args};

    #[test]
    fn parses_rust_derive_paths() {
        let derives = parse_typed_args(quote!(rust_derive(Debug, path::CustomDerive))).unwrap();
        let derives = derives
            .iter()
            .map(|path| path.to_token_stream().to_string())
            .collect::<Vec<_>>();

        assert_eq!(derives, ["Debug", "path :: CustomDerive"]);
    }

    #[test]
    fn rejects_invalid_typed_options() {
        assert!(parse_typed_args(quote!(rust_derive())).is_err());
        assert!(parse_typed_args(quote!(unknown(Debug))).is_err());
        assert!(parse_typed_args(quote!(rust_derive(Debug), rust_derive(Hash))).is_err());
        assert!(parse_typed_args(quote!(derive = (Debug, Hash))).is_err());
    }

    fn attributes(item: syn::ItemStruct) -> syn::Result<()> {
        check_serde_attributes(&item.attrs)
    }

    #[test]
    fn a_serde_attribute_inside_cfg_attr_is_rejected() {
        for item in [
            syn::parse_quote!(
                #[cfg_attr(all(), serde(skip))]
                struct S;
            ),
            // Wire-neutral where it is unconditional, but not behind a condition.
            syn::parse_quote!(
                #[cfg_attr(feature = "x", serde(rename = "s"))]
                struct S;
            ),
            syn::parse_quote!(
                #[cfg_attr(all(), allow(dead_code), serde(skip))]
                struct S;
            ),
            syn::parse_quote!(
                #[cfg_attr(all(), serde(skip), allow(dead_code))]
                struct S;
            ),
            syn::parse_quote!(
                #[cfg_attr(any(a, b), cfg_attr(all(), serde(flatten)))]
                struct S;
            ),
            syn::parse_quote!(
                #[cfg_attr(a, doc = "x", cfg_attr(b, derive(Debug), cfg_attr(c, serde(skip))))]
                struct S;
            ),
        ] {
            let error = attributes(item).unwrap_err().to_string();
            assert!(
                error.contains("a `serde` attribute inside `cfg_attr`"),
                "{error}"
            );
        }
    }

    #[test]
    fn other_attributes_inside_cfg_attr_are_accepted() {
        for item in [
            syn::parse_quote!(
                #[cfg_attr(feature = "client", derive(egui_states::Atomic))]
                struct S;
            ),
            // `serde` as the condition, a value, or part of another attribute.
            syn::parse_quote!(
                #[cfg_attr(serde, derive(Debug))]
                struct S;
            ),
            syn::parse_quote!(
                #[cfg_attr(feature = "serde", derive(serde::Serialize))]
                struct S;
            ),
            syn::parse_quote!(
                #[cfg_attr(all(), doc = "serde(skip)", allow(serde))]
                struct S;
            ),
            syn::parse_quote!(
                #[cfg_attr(any(serde, b), cfg_attr(serde, repr(C)))]
                struct S;
            ),
        ] {
            attributes(item).unwrap();
        }
    }

    /// The macro sees a member whatever its `cfg`, so `Typed` would describe
    /// a field or a variant that a build does not have.
    #[test]
    fn a_cfg_on_a_field_or_a_variant_is_rejected() {
        for mut item in [
            syn::parse_quote!(
                struct S {
                    value: u8,
                    #[cfg(feature = "x")]
                    extra: u8,
                }
            ),
            syn::parse_quote!(
                enum E {
                    A,
                    #[cfg(target_arch = "wasm32")]
                    B,
                }
            ),
            syn::parse_quote!(
                enum E {
                    Go {
                        #[cfg(any())]
                        hidden: u8,
                    },
                }
            ),
            syn::parse_quote!(
                enum E {
                    Go(#[cfg(all())] u8),
                }
            ),
            syn::parse_quote!(
                struct S {
                    #[cfg_attr(feature = "x", cfg(feature = "y"))]
                    extra: u8,
                }
            ),
            syn::parse_quote!(
                struct S {
                    #[cfg_attr(a, doc = "x", cfg_attr(b, cfg(c)))]
                    extra: u8,
                }
            ),
        ] {
            let error = add_serde_attributes(&mut item).unwrap_err().to_string();
            assert!(
                error.contains("`#[cfg]` on a field or a variant"),
                "{error}"
            );
        }
    }

    #[test]
    fn a_cfg_on_the_whole_type_and_other_conditions_are_accepted() {
        for mut item in [
            syn::parse_quote!(
                #[cfg(feature = "x")]
                struct S {
                    value: u8,
                }
            ),
            syn::parse_quote!(
                #[cfg_attr(feature = "x", cfg(feature = "y"))]
                enum E {
                    A,
                }
            ),
            // `cfg` as a condition, a value or part of another attribute.
            syn::parse_quote!(
                struct S {
                    #[cfg_attr(cfg, doc = "cfg(x)")]
                    #[doc = "cfg"]
                    value: u8,
                }
            ),
            syn::parse_quote!(
                enum E {
                    #[cfg_attr(feature = "docs", doc = "A variant.")]
                    A,
                }
            ),
        ] {
            add_serde_attributes(&mut item).unwrap();
        }
    }

    #[test]
    fn cfg_attr_is_checked_on_fields_and_variants() {
        for mut item in [
            syn::parse_quote!(
                struct S {
                    #[cfg_attr(all(), serde(skip))]
                    hidden: u8,
                }
            ),
            syn::parse_quote!(
                enum E {
                    #[cfg_attr(all(), serde(skip))]
                    Hidden,
                }
            ),
            syn::parse_quote!(
                enum E {
                    Go {
                        #[cfg_attr(all(), serde(skip))]
                        hidden: u8,
                    },
                }
            ),
            syn::parse_quote!(
                enum E {
                    Go(#[cfg_attr(all(), serde(skip))] u8),
                }
            ),
        ] {
            let error = add_serde_attributes(&mut item).unwrap_err().to_string();
            assert!(error.contains("inside `cfg_attr`"), "{error}");
        }
    }
}
