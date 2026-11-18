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
        attrs,
        ident,
        generics,
        fields,
        ..
    } = input;

    if let Some(attr) = attrs.iter().find(|attr| attr.path().is_ident("state")) {
        return Err(syn::Error::new_spanned(
            attr,
            "`state` attributes belong on fields, not on the struct",
        ));
    }

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

    // Hygienic, so an `init` expression cannot name the creator, and a user
    // item called `c` stays reachable from it.
    let c = syn::Ident::new("c", proc_macro2::Span::mixed_site());

    let initializers = fields
        .iter()
        .map(|field| field_initializer(field, &c))
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        impl egui_states::State for #ident {
            const NAME: &'static str = stringify!(#ident);

            fn new(#c: &mut impl egui_states::StatesCreator) -> Self {
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

/// State handles that `#[state(queue)]` applies to.
const QUEUE_HANDLES: [&str; 3] = ["Value", "ValueAtomic", "Signal"];

/// State handles that `#[state(init = ...)]` applies to.
const INIT_HANDLES: [&str; 4] = ["Value", "ValueAtomic", "Static", "StaticAtomic"];

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
    queue: Option<syn::Path>,
    /// The `init` path and its initial value expression.
    init: Option<(syn::Path, syn::Expr)>,
}

const EXPECTED_OPTIONS: &str = "`init = <expr>`, `low_priority`, or `queue`";

fn field_options(field: &syn::Field) -> syn::Result<FieldOptions> {
    let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
    let mut options = FieldOptions::default();
    for attr in field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("state"))
    {
        let metas = parser.parse2(attr.meta.require_list()?.tokens.clone())?;
        if metas.is_empty() {
            return Err(syn::Error::new_spanned(
                attr,
                format!("empty `state` attribute; expected {EXPECTED_OPTIONS}"),
            ));
        }
        for meta in metas {
            let unknown = |path: &syn::Path| {
                syn::Error::new_spanned(
                    path,
                    format!("unknown `state` option; expected {EXPECTED_OPTIONS}"),
                )
            };
            let duplicate =
                |path: &syn::Path| syn::Error::new_spanned(path, "duplicate `state` option");
            match meta {
                Meta::Path(path) => {
                    let option = if path.is_ident("low_priority") {
                        &mut options.low_priority
                    } else if path.is_ident("queue") {
                        &mut options.queue
                    } else if path.is_ident("init") {
                        return Err(syn::Error::new_spanned(
                            path,
                            "`init` takes a value: `init = <expr>`",
                        ));
                    } else {
                        return Err(unknown(&path));
                    };
                    if option.is_some() {
                        return Err(duplicate(&path));
                    }
                    *option = Some(path);
                }
                Meta::NameValue(meta) if meta.path.is_ident("init") => {
                    if options.init.is_some() {
                        return Err(duplicate(&meta.path));
                    }
                    options.init = Some((meta.path, meta.value));
                }
                Meta::NameValue(meta)
                    if meta.path.is_ident("low_priority") || meta.path.is_ident("queue") =>
                {
                    return Err(syn::Error::new_spanned(
                        meta,
                        "`low_priority` and `queue` take no value",
                    ));
                }
                Meta::NameValue(syn::MetaNameValue { path, .. })
                | Meta::List(syn::MetaList { path, .. }) => return Err(unknown(&path)),
            }
        }
    }
    Ok(options)
}

/// The initial value expression; a string literal is converted with `Into`
/// so that `init = "text"` works for `String` fields.
fn init_expression(expr: &syn::Expr) -> proc_macro2::TokenStream {
    // A literal forwarded through `macro_rules!` arrives in an invisible group.
    let mut inner = expr;
    while let syn::Expr::Group(group) = inner {
        inner = &group.expr;
    }

    match inner {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(_),
            ..
        }) => quote!(::core::convert::Into::into(#expr)),
        _ => quote!(#expr),
    }
}

fn field_initializer(field: &syn::Field, c: &syn::Ident) -> syn::Result<proc_macro2::TokenStream> {
    let name = field.ident.as_ref().ok_or_else(|| {
        syn::Error::new_spanned(field, "State derive requires named struct fields")
    })?;
    let field_name = name.to_string();
    let ty = &field.ty;
    let segment = last_path_segment(ty)?;
    let type_name = segment.ident.to_string();
    let options = field_options(field)?;

    let initial = match &options.init {
        Some((option, _)) if !INIT_HANDLES.contains(&type_name.as_str()) => {
            return Err(syn::Error::new_spanned(
                option,
                format!(
                    "`init` applies to {}; not to `{type_name}`",
                    INIT_HANDLES.join(", ")
                ),
            ));
        }
        Some((_, expr)) => init_expression(expr),
        None => quote!(::core::default::Default::default()),
    };

    let initializer = match type_name.as_str() {
        "Value" => quote!(#c.value(#field_name, #initial)),
        "ValueAtomic" => quote!(#c.atomic(#field_name, #initial)),
        "Static" => quote!(#c.add_static(#field_name, #initial)),
        "StaticAtomic" => quote!(#c.static_atomic(#field_name, #initial)),
        "Signal" => quote!(#c.signal(#field_name)),
        "ValueTake" => quote!(#c.value_take(#field_name)),
        "Image" => quote!(#c.image(#field_name)),
        "ImageMulti" => quote!(#c.image_multi(#field_name)),
        "MapState" => quote!(#c.map(#field_name)),
        "VecState" => quote!(#c.vec(#field_name)),
        "Data" => quote!(#c.data(#field_name)),
        "DataTake" => quote!(#c.data_take(#field_name)),
        "DataMulti" => quote!(#c.data_multi(#field_name)),
        "DataMultiTake" => quote!(#c.data_multi_take(#field_name)),
        "Download" => quote!(#c.download(#field_name)),
        "Upload" => quote!(#c.upload(#field_name)),
        _ => quote!(#c.substate(#field_name)),
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
        Some(_) => quote!(#c.low_priority(|#c| #initializer)),
        None => initializer,
    };
    let initializer = match options.queue {
        Some(option) if !QUEUE_HANDLES.contains(&type_name.as_str()) => {
            return Err(syn::Error::new_spanned(
                option,
                format!(
                    "`queue` applies to {}; not to `{type_name}`",
                    QUEUE_HANDLES.join(", ")
                ),
            ));
        }
        Some(_) => quote!(#c.queue(|#c| #initializer)),
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
