//! Procedural macros for building Astrid OS User-Space Capsules.
//!
//! This crate provides the `#[astrid::capsule]` macro to automatically
//! generate the required `extern "C"` WebAssembly exports and handle
//! seamless JSON/Binary serialization across the OS boundary.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{ImplItem, ItemImpl, LitStr};

/// Marks an `impl` block as the entry point for an Astrid Capsule.
///
/// This macro automatically generates the WebAssembly exports required by
/// the Astrid Kernel (e.g., `execute-tool`) and routes incoming IPC/Tool
/// requests to the appropriately annotated methods within the block.
#[proc_macro_attribute]
pub fn capsule(attr: TokenStream, item: TokenStream) -> TokenStream {
    capsule_impl(attr.into(), item.into()).into()
}

fn capsule_impl(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let mut input: ItemImpl = match syn::parse2(item) {
        Ok(i) => i,
        Err(e) => return e.into_compile_error(),
    };
    let struct_name = &input.self_ty.clone();

    let is_stateful = attr.to_string().trim() == "state";

    let mut tool_arms = Vec::new();
    let mut command_arms = Vec::new();
    let mut hook_arms = Vec::new();
    let mut cron_arms = Vec::new();
    let mut schema_arms = Vec::new();
    let mut install_method: Option<syn::Ident> = None;
    let mut upgrade_method: Option<syn::Ident> = None;
    let mut run_method: Option<syn::Ident> = None;

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

            // Determine if this method is marked as mutable (check before iterating)
            let is_mutable = extracted_attrs
                .iter()
                .any(|a| a.path().segments[1].ident == "mutable");

            for attr in &extracted_attrs {
                // All attrs here have exactly 2 segments (enforced by the retain
                // filter above), but guard defensively in case that changes.
                if attr.path().segments.len() < 2 {
                    continue;
                }
                let attr_name = &attr.path().segments[1].ident;

                // ---------------------------------------------------------------
                // Lifecycle hooks: install / upgrade / run
                // ---------------------------------------------------------------
                if (attr_name == "install" || attr_name == "upgrade" || attr_name == "run")
                    && is_mutable
                {
                    return syn::Error::new_spanned(
                        attr,
                        "#[astrid::mutable] cannot be used on lifecycle hooks or #[astrid::run]",
                    )
                    .into_compile_error();
                }

                if attr_name == "install" {
                    if install_method.is_some() {
                        return syn::Error::new_spanned(
                            attr,
                            "only one #[astrid::install] method is allowed per capsule",
                        )
                        .into_compile_error();
                    }
                    // Validate: no extra typed args (only &self)
                    if arg_type.is_some() {
                        return syn::Error::new_spanned(
                            &method.sig,
                            "#[astrid::install] must have signature: fn(&self) -> Result<(), SysError>",
                        )
                        .into_compile_error();
                    }
                    install_method = Some(method_name.clone());
                    continue;
                }

                if attr_name == "upgrade" {
                    if upgrade_method.is_some() {
                        return syn::Error::new_spanned(
                            attr,
                            "only one #[astrid::upgrade] method is allowed per capsule",
                        )
                        .into_compile_error();
                    }
                    // Validate: exactly one typed arg that must be &str
                    let is_ref_str = arg_type.as_ref().is_some_and(|ty| {
                        if let syn::Type::Reference(r) = ty.as_ref()
                            && let syn::Type::Path(p) = r.elem.as_ref()
                        {
                            return p.path.is_ident("str");
                        }
                        false
                    });
                    if !is_ref_str {
                        return syn::Error::new_spanned(
                            &method.sig,
                            "#[astrid::upgrade] must have signature: fn(&self, prev_version: &str) -> Result<(), SysError>",
                        )
                        .into_compile_error();
                    }
                    upgrade_method = Some(method_name.clone());
                    continue;
                }

                if attr_name == "run" {
                    if run_method.is_some() {
                        return syn::Error::new_spanned(
                            attr,
                            "only one #[astrid::run] method is allowed per capsule",
                        )
                        .into_compile_error();
                    }
                    // Validate: no extra typed args (only &self)
                    if arg_type.is_some() {
                        return syn::Error::new_spanned(
                            &method.sig,
                            "#[astrid::run] must have signature: fn(&self) -> Result<(), SysError>",
                        )
                        .into_compile_error();
                    }
                    run_method = Some(method_name.clone());
                    continue;
                }

                // ---------------------------------------------------------------
                // Existing dispatch attrs: tool / command / interceptor / cron
                // ---------------------------------------------------------------

                // Allow inferring the name from the function if no string argument is provided.
                let name_val = if let Ok(name) = attr.parse_args::<LitStr>() {
                    name.value()
                } else {
                    method_name.to_string()
                };

                let call_expr = if arg_type.is_some() {
                    quote! {
                        {
                            let args = ::serde_json::from_slice(&req.arguments)
                                .map_err(|e| ::extism_pdk::Error::msg(format!("failed to parse arguments: {}", e)))?;
                            instance.#method_name(args)?
                        }
                    }
                } else {
                    quote! {
                        instance.#method_name()?
                    }
                };

                let call_expr_stateless = if arg_type.is_some() {
                    quote! {
                        {
                            let args = ::serde_json::from_slice(&req.arguments)
                                .map_err(|e| ::extism_pdk::Error::msg(format!("failed to parse arguments: {}", e)))?;
                            get_instance().#method_name(args)?
                        }
                    }
                } else {
                    quote! {
                        get_instance().#method_name()?
                    }
                };

                let execute_block = if is_stateful {
                    quote! {
                        let mut instance: #struct_name = match ::astrid_sdk::prelude::kv::get_json("__state") {
                            Ok(state) => state,
                            Err(::astrid_sdk::SysError::JsonError(_)) => Default::default(),
                            Err(e) => return Err(::extism_pdk::Error::msg(format!("failed to load state: {}", e))),
                        };
                        let result = #call_expr;
                        ::astrid_sdk::prelude::kv::set_json("__state", &instance)
                            .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
                        let res_json = ::serde_json::to_vec(&result)
                            .map_err(|e| ::extism_pdk::Error::msg(format!("failed to serialize result: {}", e)))?;
                        return Ok(res_json);
                    }
                } else {
                    quote! {
                        let result = #call_expr_stateless;
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
                            let mut schema = ::astrid_sdk::schemars::schema_for!(#ty);
                            schema.schema.extensions.insert(
                                "mutable".to_string(),
                                ::serde_json::json!(#is_mutable),
                            );
                            map.insert(#name_val.to_string(), schema);
                        });
                    } else {
                        schema_arms.push(quote! {
                            // For parameterless tools, we generate an empty object schema
                            let mut obj = ::astrid_sdk::schemars::schema::SchemaObject {
                                instance_type: Some(::astrid_sdk::schemars::schema::SingleOrVec::Single(
                                    Box::new(::astrid_sdk::schemars::schema::InstanceType::Object)
                                )),
                                ..Default::default()
                            };
                            obj.extensions.insert(
                                "mutable".to_string(),
                                ::serde_json::json!(#is_mutable),
                            );

                            let schema = ::astrid_sdk::schemars::schema::RootSchema {
                                meta_schema: Some("http://json-schema.org/draft-07/schema#".to_string()),
                                schema: obj,
                                definitions: ::std::collections::BTreeMap::new(),
                            };
                            map.insert(#name_val.to_string(), schema);
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

    // Generate optional lifecycle exports (only when the attribute is present).
    // For stateful capsules, persist state back to KV after the hook runs.
    let install_export = install_method.map(|method_name| {
        let body = if is_stateful {
            quote! {
                // Install always starts from Default - there is no prior state
                // on first activation. The binding is mut so that interior state
                // changes (RefCell, etc.) are captured by serde serialization.
                let mut instance = #struct_name::default();
                instance.#method_name()
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
                ::astrid_sdk::prelude::kv::set_json("__state", &instance)
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
            }
        } else {
            quote! {
                let instance = #struct_name::default();
                instance.#method_name()
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
            }
        };
        quote! {
            /// WASM ABI: Called once on first install for capsule setup and elicitation.
            /// For stateful capsules, the instance is persisted to KV after install.
            /// Install always starts from `Default::default()` (no prior state exists).
            #[unsafe(no_mangle)]
            pub extern "C" fn astrid_install() -> i32 {
                // Install takes no input - the kernel sends an empty payload.
                // Input is ignored intentionally; reserved for future metadata.
                fn inner(_input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
                    #body
                    let ok = ::serde_json::to_vec(&::serde_json::json!({"ok": true}))
                        .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
                    Ok(ok)
                }
                let input = ::extism_pdk::unwrap!(::extism_pdk::input());
                let output = match inner(input) {
                    core::result::Result::Ok(x) => x,
                    core::result::Result::Err(rc) => {
                        let err = format!("{:?}", rc.0);
                        if let Ok(mut mem) = ::extism_pdk::Memory::from_bytes(&err) {
                            unsafe { ::extism_pdk::extism::error_set(mem.offset()); }
                        }
                        return rc.1;
                    }
                };
                ::extism_pdk::unwrap!(::extism_pdk::output(&output));
                0
            }
        }
    });

    let upgrade_export = upgrade_method.map(|method_name| {
        let body = if is_stateful {
            quote! {
                // JsonError covers key-not-found (host returns empty bytes which
                // fail to parse) and corrupt state - both fall back to Default.
                // HostError propagates hard - don't silently reset state on infra failures.
                let mut instance: #struct_name = match ::astrid_sdk::prelude::kv::get_json("__state") {
                    Ok(state) => state,
                    Err(e @ ::astrid_sdk::SysError::JsonError(_)) => {
                        let _ = ::astrid_sdk::log::warn(
                            &format!("failed to deserialize state, falling back to default: {}", e),
                        );
                        Default::default()
                    }
                    Err(e) => return Err(::extism_pdk::Error::msg(format!("failed to load state: {}", e))),
                };
                instance.#method_name(&req.prev_version)
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
                ::astrid_sdk::prelude::kv::set_json("__state", &instance)
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
            }
        } else {
            quote! {
                let instance = #struct_name::default();
                instance.#method_name(&req.prev_version)
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
            }
        };
        quote! {
            /// WASM ABI: Called when upgrading from a previous version.
            #[unsafe(no_mangle)]
            pub extern "C" fn astrid_upgrade() -> i32 {
                fn inner(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
                    #[derive(::serde::Deserialize)]
                    struct __AstridUpgradeRequest {
                        prev_version: String,
                    }
                    let req: __AstridUpgradeRequest = ::serde_json::from_slice(&input)
                        .map_err(|e| ::extism_pdk::Error::msg(format!("failed to parse upgrade request: {}", e)))?;
                    #body
                    let ok = ::serde_json::to_vec(&::serde_json::json!({"ok": true}))
                        .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
                    Ok(ok)
                }
                let input = ::extism_pdk::unwrap!(::extism_pdk::input());
                let output = match inner(input) {
                    core::result::Result::Ok(x) => x,
                    core::result::Result::Err(rc) => {
                        let err = format!("{:?}", rc.0);
                        if let Ok(mut mem) = ::extism_pdk::Memory::from_bytes(&err) {
                            unsafe { ::extism_pdk::extism::error_set(mem.offset()); }
                        }
                        return rc.1;
                    }
                };
                ::extism_pdk::unwrap!(::extism_pdk::output(&output));
                0
            }
        }
    });

    // Generate the run-loop export (like #[plugin_fn] pub fn run() but inside
    // the capsule impl block). For stateful capsules, state is loaded at start
    // but NOT auto-saved - run loops are long-lived and manage their own
    // persistence. For stateless capsules, delegates to the static instance.
    let run_export = run_method.map(|method_name| {
        let body = if is_stateful {
            quote! {
                let instance: #struct_name = match ::astrid_sdk::prelude::kv::get_json("__state") {
                    Ok(state) => state,
                    Err(::astrid_sdk::SysError::JsonError(_)) => Default::default(),
                    Err(e) => return Err(::extism_pdk::Error::msg(format!("failed to load state: {}", e))),
                };
                instance.#method_name()
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
            }
        } else {
            quote! {
                get_instance().#method_name()
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
            }
        };
        quote! {
            /// WASM ABI: Long-lived run loop for event-driven capsules.
            /// Generated by `#[astrid::run]` - replaces the old `#[plugin_fn] pub fn run()` pattern.
            #[unsafe(no_mangle)]
            pub extern "C" fn run() -> i32 {
                fn inner(_input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
                    #body
                    Ok(vec![])
                }
                let input = ::extism_pdk::unwrap!(::extism_pdk::input());
                let output = match inner(input) {
                    core::result::Result::Ok(x) => x,
                    core::result::Result::Err(rc) => {
                        let err = format!("{:?}", rc.0);
                        if let Ok(mut mem) = ::extism_pdk::Memory::from_bytes(&err) {
                            unsafe { ::extism_pdk::extism::error_set(mem.offset()); }
                        }
                        return rc.1;
                    }
                };
                ::extism_pdk::unwrap!(::extism_pdk::output(&output));
                0
            }
        }
    });

    // We inline the same pattern that `#[extism_pdk::plugin_fn]` would emit,
    // but with `#[doc]` attributes so that `#![warn(missing_docs)]` is satisfied
    // even when downstream crates compile with `-D warnings`.
    // `plugin_fn` strips all outer attributes and generates a bare `#[no_mangle]`
    // wrapper, losing any `#[expect(missing_docs)]` we attach — so we bypass it.

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

        /// WASM ABI: Executed by the LLM Agent via the OS Event Bus.
        #[unsafe(no_mangle)]
        pub extern "C" fn astrid_tool_call() -> i32 {
            fn inner(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
                let req: __AstridToolRequest = ::serde_json::from_slice(&input)
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
                match req.name.as_str() {
                    #( #tool_arms )*
                    _ => return Err(::extism_pdk::Error::msg("Unknown tool").into()),
                }
            }
            let input = ::extism_pdk::unwrap!(::extism_pdk::input());
            let output = match inner(input) {
                core::result::Result::Ok(x) => x,
                core::result::Result::Err(rc) => {
                    let err = format!("{:?}", rc.0);
                    if let Ok(mut mem) = ::extism_pdk::Memory::from_bytes(&err) {
                        unsafe { ::extism_pdk::extism::error_set(mem.offset()); }
                    }
                    return rc.1;
                }
            };
            ::extism_pdk::unwrap!(::extism_pdk::output(&output));
            0
        }

        /// WASM ABI: Executed by a human typing a slash-command in an Uplink (CLI/Telegram).
        #[unsafe(no_mangle)]
        pub extern "C" fn astrid_command_run() -> i32 {
            fn inner(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
                let req: __AstridToolRequest = ::serde_json::from_slice(&input)
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
                match req.name.as_str() {
                    #( #command_arms )*
                    _ => return Err(::extism_pdk::Error::msg("Unknown command").into()),
                }
            }
            let input = ::extism_pdk::unwrap!(::extism_pdk::input());
            let output = match inner(input) {
                core::result::Result::Ok(x) => x,
                core::result::Result::Err(rc) => {
                    let err = format!("{:?}", rc.0);
                    if let Ok(mut mem) = ::extism_pdk::Memory::from_bytes(&err) {
                        unsafe { ::extism_pdk::extism::error_set(mem.offset()); }
                    }
                    return rc.1;
                }
            };
            ::extism_pdk::unwrap!(::extism_pdk::output(&output));
            0
        }

        /// WASM ABI: Executed synchronously by the Kernel during OS lifecycle events (Interceptors).
        #[unsafe(no_mangle)]
        pub extern "C" fn astrid_hook_trigger() -> i32 {
            fn inner(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
                let req: __AstridToolRequest = ::serde_json::from_slice(&input)
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
                match req.name.as_str() {
                    #( #hook_arms )*
                    _ => return Err(::extism_pdk::Error::msg("Unknown hook").into()),
                }
            }
            let input = ::extism_pdk::unwrap!(::extism_pdk::input());
            let output = match inner(input) {
                core::result::Result::Ok(x) => x,
                core::result::Result::Err(rc) => {
                    let err = format!("{:?}", rc.0);
                    if let Ok(mut mem) = ::extism_pdk::Memory::from_bytes(&err) {
                        unsafe { ::extism_pdk::extism::error_set(mem.offset()); }
                    }
                    return rc.1;
                }
            };
            ::extism_pdk::unwrap!(::extism_pdk::output(&output));
            0
        }

        /// WASM ABI: Executed by the Kernel's scheduler when a static or dynamic cron job fires.
        #[unsafe(no_mangle)]
        pub extern "C" fn astrid_cron_trigger() -> i32 {
            fn inner(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
                let req: __AstridToolRequest = ::serde_json::from_slice(&input)
                    .map_err(|e| ::extism_pdk::Error::msg(e.to_string()))?;
                match req.name.as_str() {
                    #( #cron_arms )*
                    _ => return Err(::extism_pdk::Error::msg("Unknown cron job").into()),
                }
            }
            let input = ::extism_pdk::unwrap!(::extism_pdk::input());
            let output = match inner(input) {
                core::result::Result::Ok(x) => x,
                core::result::Result::Err(rc) => {
                    let err = format!("{:?}", rc.0);
                    if let Ok(mut mem) = ::extism_pdk::Memory::from_bytes(&err) {
                        unsafe { ::extism_pdk::extism::error_set(mem.offset()); }
                    }
                    return rc.1;
                }
            };
            ::extism_pdk::unwrap!(::extism_pdk::output(&output));
            0
        }

        /// WASM ABI: Auto-generated schema export for CLI builders.
        #[unsafe(no_mangle)]
        pub extern "C" fn astrid_export_schemas() -> i32 {
            fn inner(input: Vec<u8>) -> ::extism_pdk::FnResult<Vec<u8>> {
                let _ = input;
                let mut map: ::std::collections::BTreeMap<String, ::astrid_sdk::schemars::schema::RootSchema> = ::std::collections::BTreeMap::new();
                #( #schema_arms )*
                let json = ::serde_json::to_vec(&map)
                    .map_err(|e| ::extism_pdk::Error::msg(format!("failed to serialize schemas: {}", e)))?;
                Ok(json)
            }
            let input = ::extism_pdk::unwrap!(::extism_pdk::input());
            let output = match inner(input) {
                core::result::Result::Ok(x) => x,
                core::result::Result::Err(rc) => {
                    let err = format!("{:?}", rc.0);
                    if let Ok(mut mem) = ::extism_pdk::Memory::from_bytes(&err) {
                        unsafe { ::extism_pdk::extism::error_set(mem.offset()); }
                    }
                    return rc.1;
                }
            };
            ::extism_pdk::unwrap!(::extism_pdk::output(&output));
            0
        }

        #install_export
        #upgrade_export
        #run_export
    };

    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: proc_macro2::TokenStream::to_string() serialises `json!(true)` as
    // "json ! (true)" with spaces around the bang and parens. These assertions
    // rely on that stable (but undocumented) formatting.

    #[test]
    fn mutable_attr_sets_true_in_schema() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::tool("write_file")]
                #[astrid::mutable]
                fn write_file(&self, args: WriteArgs) -> Result<WriteResult, Error> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();

        assert!(
            output.contains("json ! (true)"),
            "Expected json!(true) in generated schema, but got:\n{output}"
        );
    }

    #[test]
    fn non_mutable_tool_sets_false_in_schema() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::tool("read_file")]
                fn read_file(&self, args: ReadArgs) -> Result<ReadResult, Error> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();

        assert!(
            output.contains("json ! (false)"),
            "Expected json!(false) in generated schema, but got:\n{output}"
        );
        assert!(
            !output.contains("json ! (true)"),
            "Non-mutable tool should not have json!(true)"
        );
    }

    /// `#[astrid::mutable]` listed before `#[astrid::tool]` must still work.
    #[test]
    fn mutable_before_tool_attr_order() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::mutable]
                #[astrid::tool("delete_file")]
                fn delete_file(&self, args: DeleteArgs) -> Result<DeleteResult, Error> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();

        assert!(
            output.contains("json ! (true)"),
            "Mutable-before-tool should still produce json!(true), got:\n{output}"
        );
    }

    #[test]
    fn install_generates_export() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::install]
                fn install(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("astrid_install"),
            "Expected astrid_install export, got:\n{output}"
        );
        // Should NOT generate upgrade
        assert!(
            !output.contains("astrid_upgrade"),
            "Should not generate astrid_upgrade without #[astrid::upgrade]"
        );
        // Non-stateful install should NOT persist state
        assert!(
            !output.contains("set_json"),
            "Non-stateful install should not call set_json"
        );
    }

    #[test]
    fn upgrade_generates_export() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::upgrade]
                fn upgrade(&self, prev_version: &str) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("astrid_upgrade"),
            "Expected astrid_upgrade export, got:\n{output}"
        );
        assert!(
            output.contains("__AstridUpgradeRequest"),
            "Upgrade export should generate __AstridUpgradeRequest deserialization struct"
        );
        assert!(
            output.contains("req . prev_version"),
            "Upgrade export should pass req.prev_version to the method"
        );
        // Should NOT generate install
        assert!(
            !output.contains("astrid_install"),
            "Should not generate astrid_install without #[astrid::install]"
        );
        // Non-stateful upgrade should NOT load/persist state
        assert!(
            !output.contains("set_json"),
            "Non-stateful upgrade should not call set_json"
        );
        assert!(
            !output.contains("get_json"),
            "Non-stateful upgrade should not call get_json"
        );
    }

    #[test]
    fn no_lifecycle_no_exports() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::tool("do_thing")]
                fn do_thing(&self, args: DoArgs) -> Result<String, Error> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        // Tool dispatch should still be generated
        assert!(
            output.contains("astrid_tool_call"),
            "Should still generate astrid_tool_call without lifecycle attrs"
        );
        assert!(
            !output.contains("astrid_install"),
            "Should not generate astrid_install without attribute"
        );
        assert!(
            !output.contains("astrid_upgrade"),
            "Should not generate astrid_upgrade without attribute"
        );
    }

    #[test]
    fn duplicate_install_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::install]
                fn install(&self) -> Result<(), SysError> {
                    todo!()
                }
                #[astrid::install]
                fn install2(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Duplicate #[astrid::install] should produce compile_error, got:\n{output}"
        );
    }

    #[test]
    fn duplicate_upgrade_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::upgrade]
                fn upgrade1(&self, v: &str) -> Result<(), SysError> {
                    todo!()
                }
                #[astrid::upgrade]
                fn upgrade2(&self, v: &str) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Duplicate #[astrid::upgrade] should produce compile_error, got:\n{output}"
        );
    }

    #[test]
    fn install_with_args_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::install]
                fn install(&self, args: InstallArgs) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Install with args should produce compile_error, got:\n{output}"
        );
    }

    #[test]
    fn upgrade_without_args_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::upgrade]
                fn upgrade(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Upgrade without prev_version arg should produce compile_error, got:\n{output}"
        );
    }

    #[test]
    fn upgrade_with_wrong_arg_type_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::upgrade]
                fn upgrade(&self, prev_version: u32) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Upgrade with u32 arg should produce compile_error, got:\n{output}"
        );
    }

    #[test]
    fn upgrade_with_string_arg_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::upgrade]
                fn upgrade(&self, prev_version: String) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Upgrade with String (not &str) arg should produce compile_error, got:\n{output}"
        );
    }

    #[test]
    fn both_install_and_upgrade() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::install]
                fn install(&self) -> Result<(), SysError> {
                    todo!()
                }
                #[astrid::upgrade]
                fn upgrade(&self, prev_version: &str) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("astrid_install"),
            "Should generate astrid_install"
        );
        assert!(
            output.contains("astrid_upgrade"),
            "Should generate astrid_upgrade"
        );
    }

    #[test]
    fn mutable_on_install_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::mutable]
                #[astrid::install]
                fn install(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Mutable on install should produce compile_error, got:\n{output}"
        );
    }

    #[test]
    fn mutable_on_upgrade_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::mutable]
                #[astrid::upgrade]
                fn upgrade(&self, v: &str) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Mutable on upgrade should produce compile_error, got:\n{output}"
        );
    }

    #[test]
    fn stateful_install_persists_state() {
        let attr = quote::quote! { state };
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::install]
                fn install(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("astrid_install"),
            "Should generate astrid_install"
        );
        // Stateful install must persist state to KV
        assert!(
            output.contains("set_json"),
            "Stateful install should persist state via set_json, got:\n{output}"
        );
    }

    #[test]
    fn stateful_upgrade_loads_and_persists_state() {
        let attr = quote::quote! { state };
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::upgrade]
                fn upgrade(&self, prev_version: &str) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("astrid_upgrade"),
            "Should generate astrid_upgrade"
        );
        // Stateful upgrade must load existing state from KV
        assert!(
            output.contains("get_json"),
            "Stateful upgrade should load state via get_json, got:\n{output}"
        );
        // And persist it back
        assert!(
            output.contains("set_json"),
            "Stateful upgrade should persist state via set_json, got:\n{output}"
        );
    }

    #[test]
    fn stateful_both_install_and_upgrade() {
        let attr = quote::quote! { state };
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::install]
                fn install(&self) -> Result<(), SysError> {
                    todo!()
                }
                #[astrid::upgrade]
                fn upgrade(&self, prev_version: &str) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("astrid_install"),
            "Should generate astrid_install"
        );
        assert!(
            output.contains("astrid_upgrade"),
            "Should generate astrid_upgrade"
        );
        // Both must persist state
        let install_pos = output
            .find("astrid_install")
            .expect("astrid_install missing");
        let upgrade_pos = output
            .find("astrid_upgrade")
            .expect("astrid_upgrade missing");
        // set_json must appear after both export names (in their respective bodies)
        let after_install = &output[install_pos..];
        assert!(
            after_install.contains("set_json"),
            "Stateful install must call set_json"
        );
        let after_upgrade = &output[upgrade_pos..];
        assert!(
            after_upgrade.contains("set_json"),
            "Stateful upgrade must call set_json"
        );
    }

    #[test]
    fn install_then_mutable_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::install]
                #[astrid::mutable]
                fn install(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Install-then-mutable order should also produce compile_error, got:\n{output}"
        );
    }

    /// Multiple tools in one impl block — only the mutable one gets `true`.
    #[test]
    fn multi_tool_mixed_mutability() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::tool("read_file")]
                fn read_file(&self, args: ReadArgs) -> Result<ReadResult, Error> {
                    todo!()
                }

                #[astrid::tool("write_file")]
                #[astrid::mutable]
                fn write_file(&self, args: WriteArgs) -> Result<WriteResult, Error> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();

        // Both json!(false) and json!(true) must appear — one per tool
        assert!(
            output.contains("json ! (false)"),
            "read_file should have json!(false), got:\n{output}"
        );
        assert!(
            output.contains("json ! (true)"),
            "write_file should have json!(true), got:\n{output}"
        );
    }

    // ---------------------------------------------------------------
    // #[astrid::run] tests
    // ---------------------------------------------------------------

    #[test]
    fn run_generates_export() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::run]
                fn run(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("fn run"),
            "Expected run export, got:\n{output}"
        );
        // Should NOT generate lifecycle exports
        assert!(
            !output.contains("astrid_install"),
            "Should not generate astrid_install without #[astrid::install]"
        );
    }

    #[test]
    fn run_stateless_uses_get_instance() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::run]
                fn run(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("get_instance"),
            "Stateless run should use get_instance(), got:\n{output}"
        );
    }

    #[test]
    fn run_stateful_loads_state() {
        let attr = quote::quote! { state };
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::run]
                fn run(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("get_json"),
            "Stateful run should load state via get_json, got:\n{output}"
        );
        // Run loops are infinite - should NOT auto-save state
        // Count occurrences of set_json - there should be none in the run export
        let run_pos = output.find("fn run").expect("run export missing");
        let after_run = &output[run_pos..];
        assert!(
            !after_run.contains("set_json"),
            "Stateful run should NOT auto-save state (run loops are infinite), got:\n{output}"
        );
    }

    #[test]
    fn duplicate_run_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::run]
                fn run(&self) -> Result<(), SysError> {
                    todo!()
                }
                #[astrid::run]
                fn run2(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Duplicate #[astrid::run] should produce compile_error, got:\n{output}"
        );
    }

    #[test]
    fn run_with_args_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::run]
                fn run(&self, args: RunArgs) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Run with args should produce compile_error, got:\n{output}"
        );
    }

    #[test]
    fn mutable_on_run_is_compile_error() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::mutable]
                #[astrid::run]
                fn run(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("compile_error"),
            "Mutable on run should produce compile_error, got:\n{output}"
        );
    }

    #[test]
    fn run_with_tools_and_install() {
        let attr = quote::quote! {};
        let input = quote::quote! {
            impl MyCapsule {
                #[astrid::tool("search")]
                fn search(&self, args: SearchArgs) -> Result<SearchResult, Error> {
                    todo!()
                }

                #[astrid::install]
                fn install(&self) -> Result<(), SysError> {
                    todo!()
                }

                #[astrid::run]
                fn run(&self) -> Result<(), SysError> {
                    todo!()
                }
            }
        };

        let output = capsule_impl(attr, input).to_string();
        assert!(
            output.contains("astrid_tool_call"),
            "Should generate tool dispatch"
        );
        assert!(
            output.contains("astrid_install"),
            "Should generate install export"
        );
        assert!(output.contains("fn run"), "Should generate run export");
    }
}
