use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Expr, ExprLit, Lit};

pub(crate) fn main_state_impl(attr: TokenStream, item: TokenStream) -> TokenStream {}
