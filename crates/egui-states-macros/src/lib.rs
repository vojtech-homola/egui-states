use proc_macro::TokenStream;

mod objects;
mod states;

#[proc_macro_derive(Typed)]
pub fn typed(input: TokenStream) -> TokenStream {
    objects::impl_typed(input)
}

#[proc_macro_derive(InitialValue)]
pub fn initial_value(input: TokenStream) -> TokenStream {
    objects::impl_initial_value(input)
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
