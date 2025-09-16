use proc_macro::TokenStream;

mod objects;
mod pyobjects;

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
    pyobjects::impl_pystruct(input)
}

#[proc_macro_attribute]
pub fn pyenum(_: TokenStream, input: TokenStream) -> TokenStream {
    pyobjects::impl_pyenum(input)
}

#[proc_macro_attribute]
pub fn state_enum(_: TokenStream, input: TokenStream) -> TokenStream {
    objects::impl_enum(input)
}

#[proc_macro_attribute]
pub fn state_struct(_: TokenStream, input: TokenStream) -> TokenStream {
    objects::impl_struct(input)
}