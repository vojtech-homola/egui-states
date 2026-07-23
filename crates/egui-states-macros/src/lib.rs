use proc_macro::TokenStream;

mod objects;
mod states;

#[proc_macro_derive(Transportable)]
pub fn transportable(input: TokenStream) -> TokenStream {
    objects::impl_transportable(input)
}

#[proc_macro_derive(Atomic)]
pub fn atomic(input: TokenStream) -> TokenStream {
    objects::impl_atomic(input)
}

#[proc_macro_derive(AtomicStatic)]
pub fn atomic_static(input: TokenStream) -> TokenStream {
    objects::impl_atomic_static(input)
}

#[proc_macro_derive(State)]
pub fn state(input: TokenStream) -> TokenStream {
    states::impl_state(input)
}
