use proc_macro::TokenStream;
use quote::quote;
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
    let info = parse_enum_input(input);
    let ident = &info.ident;

    let Some((names, values)) = info.fieldless() else {
        return impl_data_enum_typed(&info, rust_derives);
    };
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

fn impl_data_enum_typed(info: &EnumInfo, rust_derives: &[Path]) -> TokenStream {
    let ident = &info.ident;
    let variants = info.variants.iter().map(|variant| {
        let name = &variant.ident;
        let fields = match &variant.shape {
            VariantShape::Unit => quote!(egui_states::VariantFields::Unit),
            VariantShape::Named(fields) => {
                let names = fields.iter().map(|(name, _)| name);
                let types = fields.iter().map(|(_, ty)| ty);
                quote!(egui_states::VariantFields::Named(vec![
                    #((stringify!(#names).to_string(), <#types as egui_states::Typed>::get_type())),*
                ]))
            }
            VariantShape::Tuple(types) => quote!(egui_states::VariantFields::Tuple(vec![
                #(<#types as egui_states::Typed>::get_type()),*
            ])),
        };
        quote!((stringify!(#name).to_string(), #fields))
    });
    let field_types = info.field_types();

    let out = quote!(
        unsafe impl egui_states::Typed for #ident {
            #[inline]
            fn get_type() -> egui_states::ObjectType {
                egui_states::ObjectType::DataEnum(
                    stringify!(#ident).to_string(),
                    vec![#(#variants),*]
                )
            }

            #[doc(hidden)]
            fn rust_derives() -> Vec<egui_states::RustDerive> {
                let mut derives = vec![egui_states::RustDerive::Enum(
                    stringify!(#ident),
                    &[#(stringify!(#rust_derives)),*],
                )];
                #(derives.extend(<#field_types as egui_states::Typed>::rust_derives());)*
                derives
            }
        }
    );

    out.into()
}

fn impl_enum_initial_value(input: TokenStream) -> TokenStream {
    let info = parse_enum_input(input);
    let ident = &info.ident;

    let Some((names, _)) = info.fieldless() else {
        return impl_data_enum_initial_value(&info);
    };

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

fn impl_data_enum_initial_value(info: &EnumInfo) -> TokenStream {
    let ident = &info.ident;
    let arms = info.variants.iter().map(|variant| {
        let name = &variant.ident;
        match &variant.shape {
            VariantShape::Unit => quote!(Self::#name => (stringify!(#name), Vec::new())),
            VariantShape::Named(fields) => {
                let names = fields.iter().map(|(name, _)| name).collect::<Vec<_>>();
                quote!(Self::#name { #(#names),* } => (
                    stringify!(#name),
                    vec![#(egui_states::InitialValue::init_value(#names)),*],
                ))
            }
            VariantShape::Tuple(types) => {
                let bindings = (0..types.len())
                    .map(|index| quote::format_ident!("field_{index}"))
                    .collect::<Vec<_>>();
                quote!(Self::#name(#(#bindings),*) => (
                    stringify!(#name),
                    vec![#(egui_states::InitialValue::init_value(#bindings)),*],
                ))
            }
        }
    });

    let out = quote!(
        unsafe impl egui_states::InitialValue for #ident {
            #[inline]
            fn init_value(&self) -> egui_states::InitValue {
                let (variant, fields) = match self {
                    #(#arms),*
                };
                egui_states::InitValue::DataEnum(variant, fields)
            }
        }
    );

    out.into()
}

pub(crate) fn impl_atomic(input: TokenStream) -> TokenStream {
    let info = parse_enum_input(input);
    let ident = &info.ident;
    let Some((names, values)) = info.fieldless() else {
        panic!(
            "#[derive(Atomic)] supports only fieldless enums; implement `egui_states::Atomic` by hand for an enum with data that fits in 64 bits"
        );
    };
    let values = discriminant_expressions(&values);

    // The discriminant is stored with the bit pattern of its i32.
    let out = quote!(
        impl egui_states::Atomic for #ident {
            #[inline]
            fn to_atomic(self) -> u64 {
                self as i32 as u32 as u64
            }

            #[inline]
            fn from_atomic(bits: u64) -> Self {
                match bits as u32 as i32 {
                    #(#values => #ident::#names),*,
                    raw => panic!(
                        "Invalid enum value for {}: {}",
                        stringify!(#ident),
                        raw
                    ),
                }
            }
        }
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

/// The fields of one enum variant.
enum VariantShape {
    Unit,
    Named(Vec<(syn::Ident, syn::Type)>),
    Tuple(Vec<syn::Type>),
}

struct VariantInfo {
    ident: syn::Ident,
    shape: VariantShape,
    /// The discriminant of a variant of a fieldless enum.
    value: i32,
}

struct EnumInfo {
    ident: syn::Ident,
    variants: Vec<VariantInfo>,
}

impl EnumInfo {
    /// The variant names and discriminants, unless a variant has data.
    fn fieldless(&self) -> Option<(Vec<&syn::Ident>, Vec<i32>)> {
        self.variants
            .iter()
            .all(|variant| matches!(variant.shape, VariantShape::Unit))
            .then(|| {
                self.variants
                    .iter()
                    .map(|variant| (&variant.ident, variant.value))
                    .unzip()
            })
    }

    /// The types of every field of every variant.
    fn field_types(&self) -> Vec<&syn::Type> {
        self.variants
            .iter()
            .flat_map(|variant| match &variant.shape {
                VariantShape::Unit => Vec::new(),
                VariantShape::Named(fields) => fields.iter().map(|(_, ty)| ty).collect(),
                VariantShape::Tuple(types) => types.iter().collect(),
            })
            .collect()
    }
}

fn parse_enum_input(input: TokenStream) -> EnumInfo {
    let input = syn::parse::<syn::ItemEnum>(input)
        .unwrap_or_else(|error| panic!("Enum derive input is invalid: {error}"));
    parse_enum(input).unwrap_or_else(|error| panic!("{error}"))
}

fn parse_enum(input: syn::ItemEnum) -> Result<EnumInfo, String> {
    let syn::ItemEnum {
        ident,
        generics,
        variants,
        ..
    } = input;

    if generics.lt_token.is_some() {
        return Err("Enums with generics are not supported".to_string());
    }

    let has_data = variants
        .iter()
        .any(|variant| variant.fields != syn::Fields::Unit);

    let mut parsed = Vec::new();
    let mut next_value = Some(0i32);
    for variant in variants {
        let name = &variant.ident;
        let shape = match variant.fields {
            syn::Fields::Unit => VariantShape::Unit,
            syn::Fields::Named(fields) if fields.named.is_empty() => {
                return Err(format!(
                    "Enum variant `{name} {{}}` has empty braces; write `{name}` without them"
                ));
            }
            syn::Fields::Unnamed(fields) if fields.unnamed.is_empty() => {
                return Err(format!(
                    "Enum variant `{name}()` has empty parentheses; write `{name}` without them"
                ));
            }
            syn::Fields::Named(fields) => VariantShape::Named(
                fields
                    .named
                    .into_iter()
                    .map(|field| (field.ident.expect("named fields have names"), field.ty))
                    .collect(),
            ),
            syn::Fields::Unnamed(fields) => {
                VariantShape::Tuple(fields.unnamed.into_iter().map(|field| field.ty).collect())
            }
        };

        if let VariantShape::Named(fields) = &shape {
            check_recursion(&ident, fields.iter().map(|(_, ty)| ty))?;
        } else if let VariantShape::Tuple(types) = &shape {
            check_recursion(&ident, types.iter())?;
        }

        let value = match &variant.discriminant {
            Some(_) if has_data => {
                return Err(format!(
                    "Enum {ident} has variants with data, so its variants cannot have explicit discriminants; the protocol sends the variant's position"
                ));
            }
            Some((_, expr)) => parse_discriminant(expr),
            None if has_data => 0,
            None => next_value.expect("Enum discriminants must fit in i32"),
        };
        next_value = value.checked_add(1);

        parsed.push(VariantInfo {
            ident: variant.ident,
            shape,
            value,
        });
    }

    Ok(EnumInfo {
        ident,
        variants: parsed,
    })
}

/// A variant field that contains the enum itself would make its type
/// description endless.
fn check_recursion<'a>(
    ident: &syn::Ident,
    types: impl Iterator<Item = &'a syn::Type>,
) -> Result<(), String> {
    fn mentions(tokens: proc_macro2::TokenStream, ident: &syn::Ident) -> bool {
        tokens.into_iter().any(|token| match token {
            proc_macro2::TokenTree::Ident(name) => name == *ident || name == "Self",
            proc_macro2::TokenTree::Group(group) => mentions(group.stream(), ident),
            _ => false,
        })
    }

    for ty in types {
        if mentions(quote!(#ty), ident) {
            return Err(format!(
                "Enum {ident} cannot contain itself; recursive types are not supported"
            ));
        }
    }
    Ok(())
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
    use super::{VariantShape, discriminant_expressions, parse_enum};
    use proc_macro2::TokenTree;
    use quote::ToTokens;

    fn parse_error(item: syn::ItemEnum) -> String {
        match parse_enum(item) {
            Ok(_) => panic!("expected the enum to be rejected"),
            Err(error) => error,
        }
    }

    #[test]
    fn data_enums_keep_their_variant_shapes() {
        let info = parse_enum(syn::parse_quote! {
            enum Command { Stop, MoveTo { x: f32, y: f32 }, Pair(f32, String) }
        })
        .unwrap();
        assert!(info.fieldless().is_none());
        assert!(matches!(info.variants[0].shape, VariantShape::Unit));
        assert!(
            matches!(&info.variants[1].shape, VariantShape::Named(fields) if fields.len() == 2)
        );
        assert!(matches!(&info.variants[2].shape, VariantShape::Tuple(types) if types.len() == 2));
        assert_eq!(info.field_types().len(), 4);

        let fieldless = parse_enum(syn::parse_quote! { enum Mode { A = 2, B } }).unwrap();
        assert_eq!(fieldless.fieldless().unwrap().1, [2, 3]);
    }

    #[test]
    fn unsupported_enum_shapes_are_rejected() {
        let error = parse_error(syn::parse_quote! { enum Command { Home {}, Go(u8) } });
        assert!(error.contains("`Home {}` has empty braces"), "{error}");

        let error = parse_error(syn::parse_quote! { enum Command { Reset(), Go(u8) } });
        assert!(error.contains("`Reset()` has empty parentheses"), "{error}");

        let error = parse_error(syn::parse_quote! { enum Command { Stop = 1, Go(u8) } });
        assert!(error.contains("explicit discriminants"), "{error}");

        for item in [
            syn::parse_quote! { enum Command { Sequence(Vec<Command>) } },
            syn::parse_quote! { enum Command { Nested { inner: Option<Box<Self>> } } },
        ] {
            let error = parse_error(item);
            assert!(error.contains("cannot contain itself"), "{error}");
        }

        let error = parse_error(syn::parse_quote! { enum Command<T> { Go(T) } });
        assert!(error.contains("generics"), "{error}");
    }

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
