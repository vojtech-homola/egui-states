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
/// The attribute accepts no arguments and supports the same structs and enums
/// as the former `Typed` derive macro.
#[proc_macro_attribute]
pub fn typed(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = TokenStream2::from(args);
    if !args.is_empty() {
        return syn::Error::new_spanned(args, "`typed` does not accept arguments")
            .into_compile_error()
            .into();
    }

    let item_input = input.clone();
    let mut item = parse_macro_input!(item_input as DeriveInput);
    let typed_impl = TokenStream2::from(objects::impl_typed(input));

    match add_serde_attributes(&mut item) {
        Ok(()) => quote! {
            #item
            #typed_impl
        }
        .into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[proc_macro_derive(InitialValue)]
pub fn initial_value(input: TokenStream) -> TokenStream {
    objects::impl_initial_value(input)
}

#[proc_macro_derive(Atomic)]
pub fn atomic(input: TokenStream) -> TokenStream {
    objects::impl_atomic(input)
}

#[proc_macro_derive(AtomicStatic)]
pub fn atomic_static(input: TokenStream) -> TokenStream {
    objects::impl_atomic_static(input)
}

#[proc_macro_derive(State)]
pub fn state(input: TokenStream) -> TokenStream {
    states::impl_state(input)
}

fn add_serde_attributes(item: &mut DeriveInput) -> syn::Result<()> {
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
