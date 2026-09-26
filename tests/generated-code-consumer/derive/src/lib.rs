//! A real test-only derive, requiring no additional dependencies.
use proc_macro::{TokenStream, TokenTree};

#[proc_macro_derive(Proof)]
pub fn proof(input: TokenStream) -> TokenStream {
    let mut tokens = input.into_iter();
    while let Some(token) = tokens.next() {
        if matches!(&token, TokenTree::Ident(word) if word.to_string() == "struct") {
            let Some(TokenTree::Ident(name)) = tokens.next() else {
                panic!("expected struct name")
            };
            return format!("impl crate::GeneratedProof for {name} {{ fn proof(&self) -> &'static str {{ \"derived implementation ran\" }} }}").parse().unwrap();
        }
    }
    panic!("Proof requires a named struct")
}
