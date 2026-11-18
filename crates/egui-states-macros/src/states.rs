use proc_macro::TokenStream;
use quote::quote;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{self, Meta, Token, parse_macro_input};

pub(crate) fn impl_state(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::ItemStruct);

    match expand_state(input) {
        Ok(output) => output.into(),
        Err(err) => err.into_compile_error().into(),
    }
}

fn expand_state(input: syn::ItemStruct) -> syn::Result<proc_macro2::TokenStream> {
    let syn::ItemStruct {
        ident,
        generics,
        fields,
        ..
    } = input;

    if generics.lt_token.is_some() {
        return Err(syn::Error::new_spanned(
            generics,
            "State derive does not support generics",
        ));
    }

    let fields = match fields {
        syn::Fields::Named(fields) if fields.named.is_empty() => {
            return Err(syn::Error::new_spanned(
                ident,
                "State derive requires at least one named field",
            ));
        }
        syn::Fields::Named(fields) => fields.named,
        syn::Fields::Unnamed(fields) => {
            return Err(syn::Error::new_spanned(
                fields,
                "State derive requires named struct fields",
            ));
        }
        syn::Fields::Unit => {
            return Err(syn::Error::new_spanned(
                ident,
                "State derive requires at least one named field",
            ));
        }
    };

    let initializers = fields
        .iter()
        .map(field_initializer)
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        impl egui_states::State for #ident {
            const NAME: &'static str = stringify!(#ident);

            fn new(c: &mut impl egui_states::StatesCreator) -> Self {
                Self {
                    #(#initializers,)*
                }
            }
        }
    })
}

/// State handles that `#[state(low_priority)]` applies to. A field of any
/// other type that is not in [`HANDLES`] is a substate, where it applies to
/// these handles inside.
const BULK_HANDLES: [&str; 6] = [
    "Image",
    "ImageMulti",
    "Data",
    "DataTake",
    "DataMulti",
    "DataMultiTake",
];

/// The other state handles.
const HANDLES: [&str; 10] = [
    "Value",
    "ValueAtomic",
    "Static",
    "StaticAtomic",
    "Signal",
    "ValueTake",
    "MapState",
    "VecState",
    "Download",
    "Upload",
];

/// The options of a field's `#[state(...)]` attributes.
#[derive(Default)]
struct FieldOptions {
    low_priority: Option<syn::Path>,
}

fn field_options(field: &syn::Field) -> syn::Result<FieldOptions> {
    let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
    let mut options = FieldOptions::default();
    for attr in field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("state"))
    {
        for meta in parser.parse2(attr.meta.require_list()?.tokens.clone())? {
            let Meta::Path(path) = meta else {
                return Err(syn::Error::new_spanned(
                    meta,
                    "expected a `state` option without arguments: `low_priority`",
                ));
            };
            let option = if path.is_ident("low_priority") {
                &mut options.low_priority
            } else {
                return Err(syn::Error::new_spanned(
                    path,
                    "unknown `state` option; expected `low_priority`",
                ));
            };
            if option.is_some() {
                return Err(syn::Error::new_spanned(path, "duplicate `state` option"));
            }
            *option = Some(path);
        }
    }
    Ok(options)
}

fn field_initializer(field: &syn::Field) -> syn::Result<proc_macro2::TokenStream> {
    let name = field.ident.as_ref().ok_or_else(|| {
        syn::Error::new_spanned(field, "State derive requires named struct fields")
    })?;
    let field_name = name.to_string();
    let ty = &field.ty;
    let segment = last_path_segment(ty)?;
    let type_name = segment.ident.to_string();
    let options = field_options(field)?;

    let value_default = quote!(::core::default::Default::default());

    let initializer = match type_name.as_str() {
        "Value" => quote!(c.value(#field_name, #value_default)),
        "ValueAtomic" => quote!(c.atomic(#field_name, #value_default)),
        "Static" => quote!(c.add_static(#field_name, #value_default)),
        "StaticAtomic" => quote!(c.static_atomic(#field_name, #value_default)),
        "Signal" => quote!(c.signal(#field_name)),
        "ValueTake" => quote!(c.value_take(#field_name)),
        "Image" => quote!(c.image(#field_name)),
        "ImageMulti" => quote!(c.image_multi(#field_name)),
        "MapState" => quote!(c.map(#field_name)),
        "VecState" => quote!(c.vec(#field_name)),
        "Data" => quote!(c.data(#field_name)),
        "DataTake" => quote!(c.data_take(#field_name)),
        "DataMulti" => quote!(c.data_multi(#field_name)),
        "DataMultiTake" => quote!(c.data_multi_take(#field_name)),
        "Download" => quote!(c.download(#field_name)),
        "Upload" => quote!(c.upload(#field_name)),
        _ => quote!(c.substate(#field_name)),
    };

    let initializer = match options.low_priority {
        Some(option) if HANDLES.contains(&type_name.as_str()) => {
            return Err(syn::Error::new_spanned(
                option,
                format!(
                    "`low_priority` applies to {}, and to substates; not to `{type_name}`",
                    BULK_HANDLES.join(", ")
                ),
            ));
        }
        Some(_) => quote!(c.low_priority(|c| #initializer)),
        None => initializer,
    };

    Ok(quote!(#name: #initializer))
}

fn last_path_segment(ty: &syn::Type) -> syn::Result<&syn::PathSegment> {
    let path = match ty {
        syn::Type::Path(path) => &path.path,
        _ => {
            return Err(syn::Error::new_spanned(
                ty,
                "State derive supports only path-based field types",
            ));
        }
    };

    path.segments
        .last()
        .ok_or_else(|| syn::Error::new_spanned(ty, "State derive could not resolve the field type"))
}
