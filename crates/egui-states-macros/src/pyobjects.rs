use proc_macro::TokenStream;
use quote::quote;
use syn::{self, Lit, parse_macro_input};
// use syn::{self, parse_macro_input, Data, DeriveInput, Expr, ExprLit, Lit};

// pub(crate) fn enum_str_derive_impl(input: TokenStream) -> TokenStream {
//     let input = parse_macro_input!(input as DeriveInput);
//     let name = input.ident;

//     if let Data::Enum(d) = input.data {
//         let variants = d.variants.into_iter().map(|v| v.ident);

//         let out = quote!(
//             impl EnumStr for #name {
//                 fn as_str(&self) -> &'static str {
//                     match self {
//                         #(Self::#variants => stringify!(#variants) ),*
//                     }
//                 }
//             }
//         );

//         return out.into();
//     }

//     panic!("EnumStr can only be derived for enums");
// }

// pub(crate) fn enum_impl_derive_impl(input: TokenStream) -> TokenStream {
//     let input = parse_macro_input!(input as DeriveInput);
//     let name = input.ident;

//     if let Data::Enum(d) = input.data {
//         if d.variants.is_empty() {
//             panic!("EnumInt can not be derived for empty enums");
//         }

//         let is_some = d.variants.iter().any(|v| v.discriminant.is_some());
//         let is_all = d.variants.iter().all(|v| v.discriminant.is_some());
//         if is_some && !is_all {
//             panic!("the variant value must be specified either for none or for all variants");
//         }

//         let vals = if is_all {
//             d.variants
//                 .iter()
//                 .map(|v| {
//                     let p = v
//                         .discriminant
//                         .as_ref()
//                         .expect("the variant value must be specified either for none or for all variants")
//                         .1
//                         .clone();

//                     match p {
//                         Expr::Lit(ExprLit { lit, .. }) => match lit {
//                             Lit::Int(i) => i.base10_parse::<u64>().unwrap(),
//                             _ => panic!("the variant value must be a integer"),
//                         },
//                         _ => panic!("the variant value must be a literal"),
//                     }
//                 })
//                 .collect::<Vec<u64>>()
//         } else {
//             (0..d.variants.len() as u64).collect::<Vec<u64>>()
//         };

//         let variants = d.variants.into_iter().map(|v| v.ident);

//         let out = quote!(
//             impl EnumInt for #name {
//                 fn as_int(&self) -> u64 {
//                     *self as u64
//                 }

//                 fn from_int(value: u64) -> Result<Self, ()> {
//                     match value {
//                         #( #vals => Ok(Self::#variants), )*
//                         _ => Err(()),
//                     }
//                 }
//             }

//             impl ReadValue for #name {
//                 fn read_message(head: &[u8], data: Option<Vec<u8>>) -> Result<Self, String> {
//                     let value = u64::read_message(head, data)?;
//                     #name::from_int(value).map_err(|_| format!("Invalid enum format for enum"))
//                 }
//             }

//             impl WriteValue for #name {
//                 fn write_message(&self, head: &mut [u8]) -> Option<Vec<u8>> {
//                     unreachable!("Enum should not be written directly")
//                 }

//                 fn into_message(self) -> ValueMessage {
//                     ValueMessage::U64(self.as_int())
//                 }
//             }
//         );

//         return out.into();
//     }

//     panic!("EnumInt can only be derived for enums");
// }

// pub(crate) fn impl_state_struct(input: TokenStream) -> TokenStream {
//     let input = parse_macro_input!(input as syn::ItemStruct);

//     let syn::ItemStruct {
//         attrs,
//         vis,
//         struct_token,
//         ident,
//         generics,
//         fields,
//         semi_token,
//     } = input;

//     if generics.lt_token.is_some() {
//         panic!("Structs with generics are not supported");
//     }

//     let mut values = Vec::new();
//     let out = if let syn::Fields::Named(mut fields) = fields {
//         for field in fields.named.iter() {

//         }

//         quote!(
//             #(#attrs)*
//             #vis #struct_token #ident #fields #semi_token

//             impl egui_states::State for #ident {
//                 fn new(c: &mut egui_states::ValuesCreator) -> Self {
//                     Self {
//                         #(
//                             #fields: None,
//                         )*
//                     }
//                 }

//             }
//         )
//     } else {
//         panic!("Only named fields are supported")
//     };

//     out.into()
// }

pub(crate) fn impl_pystruct(input: TokenStream) -> TokenStream {
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

    let out = if let syn::Fields::Named(mut fields) = fields {
        for field in fields.named.iter_mut() {
            let attr: syn::Attribute = syn::parse_quote!(#[pyo3(get, set)]);
            field.attrs.push(attr);
        }

        let field_names: Vec<_> = fields.named.iter().map(|f| &f.ident).collect();
        let field_types: Vec<_> = fields.named.iter().map(|f| &f.ty).collect();

        quote!(
            #[egui_states_pyserver::pyo3::pyclass]
            #[derive(Clone, serde::Serialize, serde::Deserialize)]
            #(#attrs)*
            #vis #struct_token #ident #fields #semi_token

            #[egui_states_pyserver::pyo3::pymethods]
            impl #ident {
                #[new]
                fn new(#(#field_names: #field_types),*) -> Self {
                    Self { #(#field_names),* }
                }
            }

            impl egui_states_pyserver::ToPython for #ident {
                fn to_python<'py>(&self, py: egui_states_pyserver::pyo3::Python<'py>) -> egui_states_pyserver::pyo3::Bound<'py, egui_states_pyserver::pyo3::types::PyAny> {
                    use egui_states_pyserver::pyo3::conversion::IntoPyObjectExt;
                    self.clone().into_bound_py_any(py).unwrap()
                }
            }

            impl egui_states_pyserver::FromPython for #ident {
                fn from_python(obj: &egui_states_pyserver::pyo3::Bound<egui_states_pyserver::pyo3::PyAny>) -> egui_states_pyserver::pyo3::PyResult<Self> {
                    use egui_states_pyserver::pyo3::types::PyAnyMethods;
                    obj.extract().map_err(|e| {
                        egui_states_pyserver::pyo3::exceptions::PyValueError::new_err(format!("Failed to convert to struct: {}", e))
                    })
                }
            }
        )
    } else {
        panic!("Only named fields are supported")
    };

    out.into()
}

pub(crate) fn impl_pyenum(input: TokenStream) -> TokenStream {
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
        #[egui_states_pyserver::pyo3::pyclass(eq, hash, frozen)]
        #[derive(Hash, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
        #(#attrs)*
        #vis #enum_token #ident {
            #(#variants),*
        }

        #[egui_states_pyserver::pyo3::pymethods]
        impl #ident {
            #[new]
            fn new(value: egui_states_pyserver::EnumInit) -> egui_states_pyserver::pyo3::PyResult<Self> {
                match value {
                    egui_states_pyserver::EnumInit::Value(v) => match v {
                        #(#values => Ok(Self::#names),)*
                        _ => Err(egui_states_pyserver::pyo3::exceptions::PyValueError::new_err("Invalid enum value")),
                    },
                    egui_states_pyserver::EnumInit::Name(n) => match n.as_str() {
                        #(stringify!(#names) => Ok(Self::#names),)*
                        _ => Err(egui_states_pyserver::pyo3::exceptions::PyValueError::new_err("Invalid enum name")),
                    },
                }
            }

            #[getter]
            fn name(&self) -> &'static str {
                match self {
                    #(Self::#names => stringify!(#names),)*
                }
            }

            #[getter]
            fn value(&self) -> i64 {
                match self {
                    #(Self::#names => #values,)*
                }
            }
        }

        impl egui_states_pyserver::ToPython for #ident {
            fn to_python<'py>(&self, py: egui_states_pyserver::pyo3::Python<'py>) -> egui_states_pyserver::pyo3::Bound<'py, egui_states_pyserver::pyo3::types::PyAny> {
                use egui_states_pyserver::pyo3::conversion::IntoPyObjectExt;
                self.into_bound_py_any(py).unwrap()
            }
        }

        impl egui_states_pyserver::FromPython for #ident {
            fn from_python(obj: &egui_states_pyserver::pyo3::Bound<egui_states_pyserver::pyo3::PyAny>) -> egui_states_pyserver::pyo3::PyResult<Self> {
                use egui_states_pyserver::pyo3::types::PyAnyMethods;
                obj.extract().map_err(|e| {
                    egui_states_pyserver::pyo3::exceptions::PyValueError::new_err(format!("Failed to convert to enum: {}", e))
                })
            }
        }
    );

    out.into()
}
