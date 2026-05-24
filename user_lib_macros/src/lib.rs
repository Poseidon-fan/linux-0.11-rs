//! Procedural macros for the user-space support library.

use proc_macro::{TokenStream, TokenTree};

/// Marks a user-space Rust function as the process entry point.
///
/// The attribute keeps the function body unchanged and appends a call to
/// `user_lib::__user_lib_entry!(name)`, which emits the low-level `_start`
/// shim.
#[proc_macro_attribute]
pub fn main(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return compile_error("#[user_lib::main] does not accept arguments");
    }

    let Some(name) = function_name(&item) else {
        return compile_error("#[user_lib::main] must be applied to a function");
    };

    let mut output = item.to_string();
    output.push_str("\nuser_lib::__user_lib_entry!(");
    output.push_str(&name);
    output.push_str(");");

    output
        .parse()
        .unwrap_or_else(|_| compile_error("#[user_lib::main] failed to generate an entry point"))
}

/// Extracts the identifier immediately following the `fn` keyword.
fn function_name(item: &TokenStream) -> Option<String> {
    let mut saw_fn = false;

    for token in item.clone() {
        match token {
            TokenTree::Ident(ident) if saw_fn => return Some(ident.to_string()),
            TokenTree::Ident(ident) if ident.to_string() == "fn" => saw_fn = true,
            _ => {}
        }
    }

    None
}

/// Builds a compiler error token stream.
fn compile_error(message: &str) -> TokenStream {
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    format!("compile_error!(\"{}\");", escaped)
        .parse()
        .expect("compile_error token stream is valid")
}
