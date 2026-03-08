use proc_macro::TokenStream;

mod objects;

#[proc_macro_derive(StateEnum)]
pub fn state_enum(input: TokenStream) -> TokenStream {
    objects::impl_enum(input)
}

#[proc_macro_derive(StateStruct)]
pub fn state_struct(input: TokenStream) -> TokenStream {
    objects::impl_struct(input)
}
