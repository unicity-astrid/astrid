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
use syn::{ImplItem, ItemImpl, LitStr, parse_macro_input};

/// Marks an `impl` block as the entry point for an Astrid Capsule.
///
/// This macro automatically generates the WebAssembly exports required by
/// the Astrid Kernel (e.g., `execute-tool`) and routes incoming IPC/Tool
/// requests to the appropriately annotated methods within the block.
#[proc_macro_attribute]
pub fn capsule(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemImpl);
    let struct_name = &input.self_ty.clone();

    let is_stateful = attr.to_string().trim() == "state";

    let mut tool_arms = Vec::new();
    let mut command_arms = Vec::new();
    let mut hook_arms = Vec::new();
    let mut cron_arms = Vec::new();
    let mut schema_arms = Vec::new();

    for item in &mut input.items {
        if let ImplItem::Fn(method) = item {
            let method_name = &method.sig.ident;

            // Extract the argument type (the first Typed argument) for schema generation
            let mut arg_type = None;
            for arg in &method.sig.inputs {
                if let syn::FnArg::Typed(pat_type) = arg {
                    arg_type = Some(pat_type.ty.clone());
                    break;
                }
            }

            // Extract and process astrid attributes, then remove them
            let mut extracted_attrs = Vec::new();
            method.attrs.retain(|attr| {
                if attr.path().segments.len() == 2 && attr.path().segments[0].ident == "astrid" {
                    extracted_attrs.push(attr.clone());
                    false // Remove from the AST
                } else {
                    true // Keep other attributes
                }
            });

            for attr in extracted_attrs {
                let attr_name = &attr.path().segments[1].ident;
                if let Ok(name) = attr.parse_args::<LitStr>() {
                    let name_val = name.value();

                    let execute_block = if is_stateful {
                        quote! {
                            let args = ::serde_json::from_slice(&req.arguments)
                                .map_err(|e| ::extism_pdk::Error::msg(format!("failed to parse arguments: {}", e)))?;
                            let mut instance: #struct_name = match ::astrid_sdk::prelude::kv::get_json("__state") {
                                Ok(state) => state,
                                Err(::astrid_sdk::SysError::JsonError(_)) => Default::default(),
                                Err(e) => return Err(::extism_pdk::Error::msg(format!("failed to load state: {}", e))),
                            };
                            let result = instance.#method_name(args)?;
                            ::astrid_sdk::prelude::kv::set_json("__state", &instance)
                                .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
                            let res_json = ::serde_json::to_vec(&result)
                                .map_err(|e| ::extism_pdk::Error::msg(format!("failed to serialize result: {}", e)))?;
                            return Ok(res_json);
                        }
                    } else {
                        quote! {
                            let args = ::serde_json::from_slice(&req.arguments)
                                .map_err(|e| ::extism_pdk::Error::msg(format!("failed to parse arguments: {}", e)))?;
                            let result = get_instance().#method_name(args)?;
                            let res_json = ::serde_json::to_vec(&result)
                                .map_err(|e| ::extism_pdk::Error::msg(format!("failed to serialize result: {}", e)))?;
                            return Ok(res_json);
                        }
                    };

                    if attr_name == "tool" {
                        tool_arms.push(quote! {
                            #name_val => { #execute_block }
                        });

                        // Automatically generate schemars extraction for this tool
                        if let Some(ty) = &arg_type {
                            schema_arms.push(quote! {
                                map.insert(#name_val.to_string(), ::astrid_sdk::schemars::schema_for!(#ty));
                            });
                        }
                    } else if attr_name == "command" {
                        command_arms.push(quote! {
                            #name_val => { #execute_block }
                        });
                    } else if attr_name == "interceptor" {
                        hook_arms.push(quote! {
                            #name_val => { #execute_block }
                        });
                    } else if attr_name == "cron" {
                        cron_arms.push(quote! {
                            #name_val => { #execute_block }
                        });
                    }
                }
            }
        }
    }

    let instance_block = if is_stateful {
        quote! {}
    } else {
        quote! {
            static INSTANCE: ::std::sync::OnceLock<#struct_name> = ::std::sync::OnceLock::new();

            fn get_instance() -> &'static #struct_name {
                INSTANCE.get_or_init(|| #struct_name::default())
            }
        }
    };

    let expanded = quote! {
        #input

        // Enforce Default implementation with a clearer compiler error
        const _: () = {
            fn assert_default<T: ::std::default::Default>() {}
            let _ = assert_default::<#struct_name>;
        };

        // -------------------------------------------------------------------
        // The Astrid OS Inbound ABI
        // -------------------------------------------------------------------

        #[derive(::serde::Deserialize)]
        struct __AstridToolRequest {
            name: String,
            #[serde(default)]
            arguments: Vec<u8>,
        }

        #instance_block

        /// Executed by the LLM Agent via the OS Event Bus.
        #[allow(missing_docs)]
        #[::extism_pdk::plugin_fn]
        pub fn astrid_tool_call(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
            let req: __AstridToolRequest = ::serde_json::from_slice(&input)
                .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;

            match req.name.as_str() {
                #( #tool_arms )*
                _ => return Err(::extism_pdk::Error::msg("Unknown tool").into()),
            }
        }

        /// Executed by a human typing a slash-command in an Uplink (CLI/Telegram).
        #[allow(missing_docs)]
        #[::extism_pdk::plugin_fn]
        pub fn astrid_command_run(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
            let req: __AstridToolRequest = ::serde_json::from_slice(&input)
                .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;

            match req.name.as_str() {
                #( #command_arms )*
                _ => return Err(::extism_pdk::Error::msg("Unknown command").into()),
            }
        }

        /// Executed synchronously by the Kernel during OS lifecycle events (Interceptors).
        #[allow(missing_docs)]
        #[::extism_pdk::plugin_fn]
        pub fn astrid_hook_trigger(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
            let req: __AstridToolRequest = ::serde_json::from_slice(&input)
                .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;

            match req.name.as_str() {
                #( #hook_arms )*
                _ => return Err(::extism_pdk::Error::msg("Unknown hook").into()),
            }
        }

        /// Executed by the Kernel's scheduler when a static or dynamic cron job fires.
        #[allow(missing_docs)]
        #[::extism_pdk::plugin_fn]
        pub fn astrid_cron_trigger(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
            let req: __AstridToolRequest = ::serde_json::from_slice(&input)
                .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;

            match req.name.as_str() {
                #( #cron_arms )*
                _ => return Err(::extism_pdk::Error::msg("Unknown cron job").into()),
            }
        }

        /// Auto-generated schema export for CLI builders.
        /// Extracts all JSON schemas for tools defined in this capsule.
        #[allow(missing_docs)]
        #[::extism_pdk::plugin_fn]
        pub fn astrid_export_schemas(_input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
            let mut map: ::std::collections::HashMap<String, ::astrid_sdk::schemars::schema::RootSchema> = ::std::collections::HashMap::new();
            #( #schema_arms )*
            let json = ::serde_json::to_vec(&map).unwrap_or_default();
            Ok(json)
        }
    };

    TokenStream::from(expanded)
}
