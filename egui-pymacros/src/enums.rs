use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Expr, ExprLit, Lit};

pub(crate) fn enum_str_derive_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;

    if let Data::Enum(d) = input.data {
        let variants = d.variants.into_iter().map(|v| v.ident);

        let out = quote!(
            impl EnumStr for #name {
                fn as_str(&self) -> &'static str {
                    match self {
                        #(Self::#variants => stringify!(#variants) ),*
                    }
                }
            }
        );

        return out.into();
    }

    panic!("EnumStr can only be derived for enums");
}

pub(crate) fn enum_int_derive_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;

    if let Data::Enum(d) = input.data {
        if d.variants.is_empty() {
            panic!("EnumInt can not be derived for empty enums");
        }

        let is_some = d.variants.iter().any(|v| v.discriminant.is_some());
        let is_all = d.variants.iter().all(|v| v.discriminant.is_some());
        if is_some && !is_all {
            panic!("the variant value must be specified either for none or for all variants");
        }

        let vals = if is_all {
            d.variants
                .iter()
                .map(|v| {
                    let p = v
                        .discriminant
                        .as_ref()
                        .expect("the variant value must be specified either for none or for all variants")
                        .1
                        .clone();

                    match p {
                        Expr::Lit(ExprLit { lit, .. }) => match lit {
                            Lit::Int(i) => i.base10_parse::<u64>().unwrap(),
                            _ => panic!("the variant value must be a integer"),
                        },
                        _ => panic!("the variant value must be a literal"),
                    }
                })
                .collect::<Vec<u64>>()
        } else {
            (0..d.variants.len() as u64).collect::<Vec<u64>>()
        };

        let variants = d.variants.into_iter().map(|v| v.ident);

        let out = quote!(
            impl EnumInt for #name {
                fn as_int(&self) -> u64 {
                    *self as u64
                }

                fn from_int(value: u64) -> Result<Self, ()> {
                    match value {
                        #( #vals => Ok(Self::#variants), )*
                        _ => Err(()),
                    }
                }
            }
        );

        return out.into();
    }

    panic!("EnumInt can only be derived for enums");
}