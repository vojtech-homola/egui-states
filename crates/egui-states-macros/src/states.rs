use proc_macro::TokenStream;
use quote::quote;
use syn::{self, parse_macro_input};

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

fn field_initializer(field: &syn::Field) -> syn::Result<proc_macro2::TokenStream> {
    let name = field.ident.as_ref().ok_or_else(|| {
        syn::Error::new_spanned(field, "State derive requires named struct fields")
    })?;
    let field_name = name.to_string();
    let ty = &field.ty;
    let segment = last_path_segment(ty)?;
    let type_name = segment.ident.to_string();

    let value_default = quote!(::core::default::Default::default());

    let initializer = match type_name.as_str() {
        "Value" => quote!(c.value(#field_name, #value_default)),
        "ValueAtomic" => quote!(c.atomic(#field_name, #value_default)),
        "Static" => quote!(c.add_static(#field_name, #value_default)),
        "StaticAtomic" => quote!(c.static_atomic(#field_name, #value_default)),
        "Signal" => quote!(c.signal(#field_name)),
        "ValueImage" => quote!(c.image(#field_name)),
        "ValueMap" => quote!(c.map(#field_name)),
        "ValueVec" => quote!(c.vec(#field_name)),
        "ValueGraphs" => quote!(c.graphs(#field_name)),
        _ => quote!(c.substate(#field_name)),
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
