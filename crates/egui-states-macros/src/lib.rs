use proc_macro::TokenStream;

mod objects;
mod states;

#[proc_macro_derive(Transportable)]
pub fn transportable(input: TokenStream) -> TokenStream {
    objects::impl_transportable(input)
}

#[proc_macro_derive(State)]
pub fn state(input: TokenStream) -> TokenStream {
    states::impl_state(input)
}
