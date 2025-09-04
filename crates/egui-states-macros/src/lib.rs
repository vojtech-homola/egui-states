use proc_macro::TokenStream;

mod enums;

// #[proc_macro_derive(EnumStr)]
// pub fn enum_str_derive(input: TokenStream) -> TokenStream {
//     enums::enum_str_derive_impl(input)
// }

// #[proc_macro_derive(EnumInt)]
// pub fn enum_int_derive(input: TokenStream) -> TokenStream {
//     enums::enum_int_derive_impl(input)
// }

// #[proc_macro_derive(EnumImpl)]
// pub fn enum_impl_derive(input: TokenStream) -> TokenStream {
//     enums::enum_impl_derive_impl(input)
// }

#[proc_macro_attribute]
pub fn pystruct(_: TokenStream, input: TokenStream) -> TokenStream {
    enums::impl_pystruct(input)
}

#[proc_macro_attribute]
pub fn pyenum(_: TokenStream, input: TokenStream) -> TokenStream {
    enums::impl_pyenum(input)
}