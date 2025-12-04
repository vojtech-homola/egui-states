use proc_macro::TokenStream;
use quote::quote;
use syn::{self, Lit, parse_macro_input};

pub(crate) fn impl_struct(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::ItemStruct);

    let syn::ItemStruct {
        attrs,
        vis,
        struct_token,
        ident,
        generics,
        fields,
        semi_token,
    } = input;

    if generics.lt_token.is_some() {
        panic!("Structs with generics are not supported");
    }

    let fields_iter = fields.clone().into_iter().map(|f| f);
    let mut names = Vec::new();
    let mut types = Vec::new();
    for field in fields_iter {
        if let Some(ident) = &field.ident {
            names.push(ident.clone());
            types.push(field.ty.clone());
        } else {
            panic!("Struct fields must be named");
        }
    }
    // let types2 = types.clone();

    let out = quote!(
        #[derive(Clone, serde::Serialize, serde::Deserialize)]
        #(#attrs)*
        #vis #struct_token #ident #fields #semi_token

        // impl egui_states::values_info::GetTypeInfo for #ident {
        //     #[inline]
        //     fn type_info() -> egui_states::values_info::TypeInfo {
        //         egui_states::values_info::TypeInfo::Struct(stringify!(#ident) ,vec![
        //             #((stringify!(#names), <#types as egui_states::values_info::GetTypeInfo>::type_info())),*
        //         ])
        //     }
        // }

        impl egui_states::GetInitValue for #ident {
            #[inline]
            fn init_value(&self) -> egui_states::InitValue {
                egui_states::InitValue::Tuple(vec![#(self.#names.init_value()),*])
            }
        }

        impl egui_states::GetType for #ident {
            #[inline]
            fn get_type() -> egui_states::ObjectType {
                egui_states::ObjectType::Struct(
                    stringify!(#ident).to_string(),
                    vec![
                        #((stringify!(#names).to_string(), <#types as egui_states::GetType>::get_type())),*
                    ]
                )
            }
        }
    );

    out.into()
}

pub(crate) fn impl_enum(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::ItemEnum);

    let syn::ItemEnum {
        attrs,
        vis,
        enum_token,
        ident,
        generics,
        variants,
        ..
    } = input;

    if generics.lt_token.is_some() {
        panic!("Enums with generics are not supported");
    }

    let variants = variants.clone().into_iter().map(|v| v);
    let mut names = Vec::new();
    let mut values = Vec::new();
    let mut actual = 0i64;
    for variant in variants.clone() {
        if variant.fields != syn::Fields::Unit {
            panic!("Enum variants must be unit variants");
        }

        if let Some((_, expr)) = &variant.discriminant {
            if let syn::Expr::Lit(syn::ExprLit { lit, .. }) = expr {
                if let Lit::Int(lit) = lit {
                    let v = lit.base10_parse::<i64>().unwrap();
                    actual = v;
                } else {
                    panic!("Enum discriminants must be integers");
                }
            } else {
                panic!("Enum discriminants must be literals");
            }
        }

        names.push(variant.ident.clone());
        values.push(actual);
        actual += 1;
    }

    let out = quote!(
        #[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
        #(#attrs)*
        #vis #enum_token #ident {
            #(#variants),*
        }

        // impl egui_states::values_info::GetTypeInfo for #ident {
        //     #[inline]
        //     fn type_info() -> egui_states::values_info::TypeInfo {
        //         egui_states::values_info::TypeInfo::Enum(stringify!(#ident), vec![
        //             #((stringify!(#names), #values as isize)),*
        //         ])
        //     }
        // }

        impl egui_states::GetInitValue for #ident {
            #[inline]
            fn init_value(&self) -> egui_states::InitValue {
                egui_states::InitValue::Enum(format!("{:?}", self))
            }
        }

        impl egui_states::GetType for #ident {
            #[inline]
            fn get_type() -> egui_states::ObjectType {
                egui_states::ObjectType::Enum(
                    stringify!(#ident).to_string(),
                    vec![
                        #((stringify!(#names).to_string(), #values as isize)),*
                    ]
                )
            }
        }
    );

    out.into()
}
