use proc_macro::TokenStream;

mod enums;
mod states;

#[proc_macro_derive(EnumStr)]
pub fn enum_str_derive(input: TokenStream) -> TokenStream {
    enums::enum_str_derive_impl(input)
}

#[proc_macro_derive(EnumInt)]
pub fn enum_int_derive(input: TokenStream) -> TokenStream {
    enums::enum_int_derive_impl(input)
}

// #[proc_macro_attribute]
// pub fn main_state(attr: TokenStream, item: TokenStream) -> TokenStream {}
