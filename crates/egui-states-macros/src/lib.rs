//! Procedural macros for the `egui_states` crate.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Attribute, DeriveInput, Meta, Path, Token, parse::Parser, parse_macro_input,
    punctuated::Punctuated,
};

mod objects;
mod states;

/// Implements `egui_states::Typed` and derives Serde serialization through
/// the `egui_states` Serde re-export.
///
/// Use `rust_derive(...)` to add derives to the corresponding generated Rust
/// server type. Derive paths are emitted as written and must be available in the
/// crate compiling the generated bindings. They are not added to the source
/// type, and prerequisite derives are not inferred.
///
/// The attribute supports non-generic unit structs or structs with named
/// fields, and non-generic fieldless enums whose discriminants fit in `i32`;
/// unions, tuple structs, data-carrying enums, and generic items are not
/// supported. Every field type must implement `egui_states::Typed`.
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

/// Derives `egui_states::InitialValue` for a struct or fieldless enum.
///
/// This supports the same item shapes as [`typed`]. Every struct field must
/// implement `egui_states::InitialValue`.
#[proc_macro_derive(InitialValue)]
pub fn initial_value(input: TokenStream) -> TokenStream {
    objects::impl_initial_value(input)
}

/// Derives the client-side `egui_states::Atomic` synchronization traits.
///
/// The input must be a non-generic fieldless enum whose discriminants fit in
/// `i32`; the generated implementation stores the discriminant atomically.
#[proc_macro_derive(Atomic)]
pub fn atomic(input: TokenStream) -> TokenStream {
    objects::impl_atomic(input)
}

/// Derives `egui_states::AtomicStatic` for a copyable value.
///
/// The input must be a non-generic fieldless enum whose discriminants fit in
/// `i32`; the generated implementation stores the discriminant atomically.
#[proc_macro_derive(AtomicStatic)]
pub fn atomic_static(input: TokenStream) -> TokenStream {
    objects::impl_atomic_static(input)
}

/// Derives `egui_states::State` for a struct whose fields are state handles.
///
/// Each field is registered under its Rust field name. Nested structs that
/// also derive `State` become nested state groups. The struct must be
/// non-generic, contain at least one named field, and use path-based field
/// types. Recognized state-handle names are initialized through the matching
/// `StatesCreator` method; other field types are treated as nested states.
#[proc_macro_derive(State)]
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

fn check_serde_attributes(attrs: &[Attribute]) -> syn::Result<()> {
    let parser = Punctuated::<Meta, Token![,]>::parse_terminated;

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
                check_serde_attributes(&field.attrs)?;
            }
        }
        syn::Data::Enum(data) => {
            for variant in &data.variants {
                check_serde_attributes(&variant.attrs)?;
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

    use super::parse_typed_args;

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
}
