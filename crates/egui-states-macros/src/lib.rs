use proc_macro::TokenStream;

mod objects;

#[proc_macro_derive(Transportable)]
pub fn transportable(input: TokenStream) -> TokenStream {
    objects::impl_transportable(input)
}
