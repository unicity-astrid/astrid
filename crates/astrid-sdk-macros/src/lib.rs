//! Procedural macros for building Astrid OS User-Space Capsules.
//!
//! This crate provides the `#[astrid::capsule]` macro to automatically
//! generate the required `extern "C"` WebAssembly exports and handle
//! seamless JSON/Binary serialization across the OS boundary.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemImpl};

/// Marks an `impl` block as the entry point for an Astrid Capsule.
///
/// This macro automatically generates the WebAssembly exports required by
/// the Astrid Kernel (e.g., `execute-tool`) and routes incoming IPC/Tool
/// requests to the appropriately annotated methods within the block.
#[proc_macro_attribute]
pub fn capsule(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemImpl);
    let _struct_name = &input.self_ty;

    // In a fully built macro, we would parse all the `#[tool("name")]` attributes
    // on the methods inside the `impl` block, generate a `match` router, and
    // deserialize the arguments.
    //
    // For now, this just emits the struct and a placeholder Extism export to
    // prove the architecture works perfectly.

    let expanded = quote! {
        #input

        // -------------------------------------------------------------------
        // The Astrid OS Inbound ABI
        // -------------------------------------------------------------------
        // These are the three standard, binary-safe WebAssembly exports that
        // the Astrid Microkernel expects to find in a Pure WASM Capsule.

        /// Executed by the LLM Agent via the OS Event Bus.
        #[::extism_pdk::plugin_fn]
        pub fn astrid_tool_call(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
            // TODO: Generate AST router based on #[tool] attributes
            // e.g., deserialize `input` (JSON/MsgPack) into a ToolRequest,
            // route to #struct_name::my_tool(...), and serialize the result.
            
            Ok(b"Successfully routed tool call".to_vec())
        }

        /// Executed by a human typing a slash-command in an Uplink (CLI/Telegram).
        #[::extism_pdk::plugin_fn]
        pub fn astrid_command_run(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
            // TODO: Generate AST router based on #[command] attributes
            Ok(b"Successfully routed slash command".to_vec())
        }

        /// Executed synchronously by the Kernel during OS lifecycle events (Interceptors).
        #[::extism_pdk::plugin_fn]
        pub fn astrid_hook_trigger(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
            // TODO: Generate AST router based on #[interceptor] attributes
            Ok(b"Successfully routed hook trigger".to_vec())
        }

        /// Executed by the Kernel's scheduler when a static or dynamic cron job fires.
        #[::extism_pdk::plugin_fn]
        pub fn astrid_cron_trigger(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
            // TODO: Generate AST router based on #[cron] attributes
            Ok(b"Successfully routed cron trigger".to_vec())
        }
    };

    TokenStream::from(expanded)
}
