use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{self, Lit, parse_macro_input};

pub(crate) fn impl_struct(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::ItemStruct);

    let syn::ItemStruct {
        ident,
        generics,
        fields,
        ..
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

    let out = quote!(
        impl egui_states::GetInitValue for #ident {
            #[inline]
            fn init_value(&self) -> egui_states::InitValue {
                egui_states::InitValue::Struct(
                    stringify!(#ident),
                    vec![
                        #((stringify!(#names), self.#names.init_value())),*
                    ]
                )
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
    let mut actual = 0i32;
    for variant in variants.clone() {
        if variant.fields != syn::Fields::Unit {
            panic!("Enum variants must be unit variants");
        }

        if let Some((_, expr)) = &variant.discriminant {
            if let syn::Expr::Lit(syn::ExprLit { lit, .. }) = expr {
                if let Lit::Int(lit) = lit {
                    let v = lit
                        .base10_parse::<i32>()
                        .expect("Enum discriminants must fit in i32");
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
    let values2 = values.clone();
    let private_ident = format_ident!("__Private{}", ident);
    let private_mod = format_ident!("__private_{}", ident);

    let out = quote!(
        impl egui_states::GetInitValue for #ident {
            #[inline]
            fn init_value(&self) -> egui_states::InitValue {
                egui_states::InitValue::Enum(match self {
                    #(Self::#names => concat!(stringify!(#ident), "::", stringify!(#names)).to_string()),*
                })
            }
        }

        impl egui_states::GetType for #ident {
            #[inline]
            fn get_type() -> egui_states::ObjectType {
                egui_states::ObjectType::Enum(
                    stringify!(#ident).to_string(),
                    vec![
                        #((stringify!(#names).to_string(), #values)),*
                    ]
                )
            }
        }

        #[allow(non_snake_case)]
        mod #private_mod {
            use std::sync::atomic::AtomicI32;

            pub struct #private_ident(pub AtomicI32);
        }

        unsafe impl egui_states::AtomicLockStatic<#ident> for #private_mod::#private_ident {
            #[inline]
            fn new(value: #ident) -> Self {
                Self(std::sync::atomic::AtomicI32::new(value as i32))
            }

            #[inline]
            fn load(&self) -> #ident {
                match self.0.load(std::sync::atomic::Ordering::Acquire) {
                    #(#values2 => #ident::#names),*,
                    raw => panic!(
                        "Invalid enum value for {}: {}",
                        stringify!(#ident),
                        raw
                    ),
                }
            }

            #[inline]
            fn store(&self, value: #ident) {
                self.0.store(value as i32, std::sync::atomic::Ordering::Release);
            }
        }

        unsafe impl egui_states::AtomicStatic for #ident {
            type Lock = #private_mod::#private_ident;
        }

        unsafe impl egui_states::Atomic for #ident {
            type Lock = egui_states::UpdateLock<#private_mod::#private_ident>;
        }
    );

    out.into()
}
