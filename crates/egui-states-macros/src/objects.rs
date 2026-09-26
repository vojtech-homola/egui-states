use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{self, Lit, Path, parse_macro_input};

pub(crate) fn impl_typed(input: TokenStream, rust_derives: &[Path]) -> TokenStream {
    let input_clone = input.clone();
    let input = parse_macro_input!(input as syn::DeriveInput);

    match input.data {
        syn::Data::Struct(_) => impl_struct_typed(input_clone, rust_derives),
        syn::Data::Enum(_) => impl_enum_typed(input_clone, rust_derives),
        syn::Data::Union(_) => panic!("Unions are not supported"),
    }
}

pub(crate) fn impl_initial_value(input: TokenStream) -> TokenStream {
    let input_clone = input.clone();
    let input = parse_macro_input!(input as syn::DeriveInput);

    match input.data {
        syn::Data::Struct(_) => impl_struct_initial_value(input_clone),
        syn::Data::Enum(_) => impl_enum_initial_value(input_clone),
        syn::Data::Union(_) => panic!("Unions are not supported"),
    }
}

fn impl_struct_typed(input: TokenStream, rust_derives: &[Path]) -> TokenStream {
    let StructInfo {
        ident,
        names,
        types,
    } = parse_struct(input);

    let out = quote!(
        unsafe impl egui_states::Typed for #ident {
            #[inline]
            fn get_type() -> egui_states::ObjectType {
                egui_states::ObjectType::Struct(
                    stringify!(#ident).to_string(),
                    vec![
                        #((stringify!(#names).to_string(), <#types as egui_states::Typed>::get_type())),*
                    ]
                )
            }

            #[doc(hidden)]
            fn rust_derives() -> Vec<egui_states::RustDerive> {
                let mut derives = vec![egui_states::RustDerive::Struct(
                    stringify!(#ident),
                    &[#(stringify!(#rust_derives)),*],
                )];
                #(derives.extend(<#types as egui_states::Typed>::rust_derives());)*
                derives
            }
        }
    );

    out.into()
}

fn impl_struct_initial_value(input: TokenStream) -> TokenStream {
    let StructInfo { ident, names, .. } = parse_struct(input);

    let out = quote!(
        unsafe impl egui_states::InitialValue for #ident {
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
    );

    out.into()
}

fn impl_enum_typed(input: TokenStream, rust_derives: &[Path]) -> TokenStream {
    let EnumInfo {
        ident,
        names,
        values,
    } = parse_enum(input);
    let values = discriminant_expressions(&values);

    let out = quote!(
        unsafe impl egui_states::Typed for #ident {
            #[inline]
            fn get_type() -> egui_states::ObjectType {
                egui_states::ObjectType::Enum(
                    stringify!(#ident).to_string(),
                    vec![
                        #((stringify!(#names).to_string(), #values)),*
                    ]
                )
            }

            #[doc(hidden)]
            fn rust_derives() -> Vec<egui_states::RustDerive> {
                vec![egui_states::RustDerive::Enum(
                    stringify!(#ident),
                    &[#(stringify!(#rust_derives)),*],
                )]
            }
        }
    );

    out.into()
}

fn impl_enum_initial_value(input: TokenStream) -> TokenStream {
    let EnumInfo { ident, names, .. } = parse_enum(input);

    let out = quote!(
        unsafe impl egui_states::InitialValue for #ident {
            #[inline]
            fn init_value(&self) -> egui_states::InitValue {
                egui_states::InitValue::Enum(match self {
                    #(Self::#names => stringify!(#names).to_string()),*
                })
            }
        }
    );

    out.into()
}

pub(crate) fn impl_atomic(input: TokenStream) -> TokenStream {
    impl_enum_atomic(input, AtomicKind::Atomic)
}

pub(crate) fn impl_atomic_static(input: TokenStream) -> TokenStream {
    impl_enum_atomic(input, AtomicKind::AtomicStatic)
}

enum AtomicKind {
    Atomic,
    AtomicStatic,
}

fn impl_enum_atomic(input: TokenStream, kind: AtomicKind) -> TokenStream {
    let EnumInfo {
        ident,
        names,
        values,
    } = parse_enum(input);
    let values = discriminant_expressions(&values);

    let (private_ident, private_mod) = match kind {
        AtomicKind::Atomic => (
            format_ident!("__PrivateAtomic{}", ident),
            format_ident!("__private_atomic_{}", ident),
        ),
        AtomicKind::AtomicStatic => (
            format_ident!("__PrivateAtomicStatic{}", ident),
            format_ident!("__private_atomic_static_{}", ident),
        ),
    };

    let atomic_impl = match kind {
        AtomicKind::Atomic => quote!(
            unsafe impl egui_states::Atomic for #ident {
                type Lock = egui_states::UpdateLock<#private_mod::#private_ident>;
            }
        ),
        AtomicKind::AtomicStatic => quote!(
            unsafe impl egui_states::AtomicStatic for #ident {
                type Lock = #private_mod::#private_ident;
            }
        ),
    };

    let out = quote!(
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
                    #(#values => #ident::#names),*,
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

        #atomic_impl
    );

    out.into()
}

fn discriminant_expressions(values: &[i32]) -> Vec<syn::Expr> {
    // A negative Literal token confuses rust-analyzer in nested macro calls.
    // Parsing emits a separate unary minus, including for i32::MIN, while
    // retaining the i32 suffix used by the original integer interpolation.
    values
        .iter()
        .map(|value| {
            syn::parse_str(&format!("{value}i32"))
                .expect("an i32 always forms a valid Rust expression")
        })
        .collect()
}

struct StructInfo {
    ident: syn::Ident,
    names: Vec<syn::Ident>,
    types: Vec<syn::Type>,
}

fn parse_struct(input: TokenStream) -> StructInfo {
    let input = syn::parse::<syn::ItemStruct>(input)
        .unwrap_or_else(|error| panic!("Struct derive input is invalid: {error}"));

    let syn::ItemStruct {
        ident,
        generics,
        fields,
        ..
    } = input;

    if generics.lt_token.is_some() {
        panic!("Structs with generics are not supported");
    }

    let mut names = Vec::new();
    let mut types = Vec::new();
    for field in fields {
        if let Some(ident) = field.ident {
            names.push(ident);
            types.push(field.ty);
        } else {
            panic!("Struct fields must be named");
        }
    }

    StructInfo {
        ident,
        names,
        types,
    }
}

struct EnumInfo {
    ident: syn::Ident,
    names: Vec<syn::Ident>,
    values: Vec<i32>,
}

fn parse_enum(input: TokenStream) -> EnumInfo {
    let input = syn::parse::<syn::ItemEnum>(input)
        .unwrap_or_else(|error| panic!("Enum derive input is invalid: {error}"));

    let syn::ItemEnum {
        ident,
        generics,
        variants,
        ..
    } = input;

    if generics.lt_token.is_some() {
        panic!("Enums with generics are not supported");
    }

    let mut names = Vec::new();
    let mut values = Vec::new();
    let mut next_value = Some(0i32);
    for variant in variants {
        if variant.fields != syn::Fields::Unit {
            panic!("Enum variants must be unit variants");
        }

        let actual = if let Some((_, expr)) = &variant.discriminant {
            parse_discriminant(expr)
        } else {
            next_value.expect("Enum discriminants must fit in i32")
        };

        names.push(variant.ident.clone());
        values.push(actual);
        next_value = actual.checked_add(1);
    }

    EnumInfo {
        ident,
        names,
        values,
    }
}

fn parse_discriminant(expr: &syn::Expr) -> i32 {
    let (negative, lit) = match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: Lit::Int(lit), ..
        }) => (false, lit),
        syn::Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr,
            ..
        }) => match expr.as_ref() {
            syn::Expr::Lit(syn::ExprLit {
                lit: Lit::Int(lit), ..
            }) => (true, lit),
            _ => panic!("Enum discriminants must be integer literals"),
        },
        syn::Expr::Lit(_) => panic!("Enum discriminants must be integers"),
        _ => panic!("Enum discriminants must be integer literals"),
    };

    let magnitude = lit
        .base10_parse::<i64>()
        .expect("Enum discriminants must fit in i32");
    let value = if negative { -magnitude } else { magnitude };
    i32::try_from(value).expect("Enum discriminants must fit in i32")
}

#[cfg(test)]
mod tests {
    use super::discriminant_expressions;
    use proc_macro2::TokenTree;
    use quote::ToTokens;

    #[test]
    fn negative_discriminants_emit_a_separate_minus_token() {
        for value in [i32::MIN, -2, -1] {
            let expressions = discriminant_expressions(&[value]);
            let tokens: Vec<_> = expressions[0].to_token_stream().into_iter().collect();
            assert_eq!(tokens.len(), 2, "unexpected tokens for {value}: {tokens:?}");
            assert!(matches!(&tokens[0], TokenTree::Punct(p) if p.as_char() == '-'));
            assert!(matches!(&tokens[1], TokenTree::Literal(_)));
            assert_eq!(
                tokens[1].to_string(),
                format!("{}i32", value.unsigned_abs())
            );
        }
    }
}
