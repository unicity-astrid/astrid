//! Pure-Rust WASM export stitcher.
//!
//! Replaces Binaryen's `wasm-merge` with a targeted transformation:
//! adds named wrapper exports to a Wizer'd `QuickJS` kernel.
//!
//! The `QuickJS` kernel exports `__invoke_i32(i32) -> i32` which calls the
//! JS function at a sorted index in `module.exports`. We add named exports
//! like `describe-tools` that call `__invoke_i32(0)`, `execute-tool` that
//! calls `__invoke_i32(1)`, etc. — where the index is the alphabetical
//! position of the export name.

use crate::error::{BridgeError, BridgeResult};

/// Add named WASM exports that delegate to `__invoke_i32(index)`.
///
/// `export_names` must be provided in the order matching the sorted index
/// that the `QuickJS` kernel expects (alphabetical order of `module.exports` keys).
///
/// # Errors
///
/// Returns `BridgeError::ExportStitchFailed` if the input WASM is malformed
/// or missing the `__invoke_i32` export.
pub fn stitch_exports(wasm_bytes: &[u8], export_names: &[&str]) -> BridgeResult<Vec<u8>> {
    let info = gather_module_info(wasm_bytes)?;

    let invoke_func_idx = info.invoke_i32_func_idx.ok_or_else(|| {
        BridgeError::ExportStitchFailed(
            "__invoke_i32 export not found in kernel WASM. \
             Ensure the kernel was built from extism/js-pdk's core crate."
                .into(),
        )
    })?;

    let arg_start_func_idx = info.arg_start_func_idx.ok_or_else(|| {
        BridgeError::ExportStitchFailed(
            "__arg_start export not found in kernel WASM. \
             Ensure the kernel was built from extism/js-pdk's core crate."
                .into(),
        )
    })?;

    let needs_new_type = info.void_to_i32_type_idx.is_none();
    let wrapper_type_idx = info.void_to_i32_type_idx.unwrap_or(info.num_types);
    let total_existing_funcs = info
        .num_imported_funcs
        .checked_add(info.num_defined_funcs)
        .ok_or_else(|| BridgeError::ExportStitchFailed("function count overflow".to_string()))?;

    let ctx = StitchContext {
        wasm_bytes,
        export_names,
        invoke_func_idx,
        arg_start_func_idx,
        needs_new_type,
        wrapper_type_idx,
        total_existing_funcs,
    };

    rebuild_module(&ctx)
}

/// Parameters for the stitching pass.
struct StitchContext<'a> {
    wasm_bytes: &'a [u8],
    export_names: &'a [&'a str],
    invoke_func_idx: u32,
    arg_start_func_idx: u32,
    needs_new_type: bool,
    wrapper_type_idx: u32,
    total_existing_funcs: u32,
}

/// Rebuild the WASM module, modifying type/function/export/code sections.
fn rebuild_module(ctx: &StitchContext<'_>) -> BridgeResult<Vec<u8>> {
    let mut module = wasm_encoder::Module::new();
    let mut code_bodies: Vec<Vec<u8>> = Vec::new();
    let mut code_section_pending = false;

    for payload in wasmparser::Parser::new(0).parse_all(ctx.wasm_bytes) {
        let payload = payload
            .map_err(|e| BridgeError::ExportStitchFailed(format!("failed to parse WASM: {e}")))?;

        // Emit pending code section before the next non-code payload
        if code_section_pending && !matches!(payload, wasmparser::Payload::CodeSectionEntry(_)) {
            emit_code_section(
                &mut module,
                &code_bodies,
                ctx.export_names,
                ctx.arg_start_func_idx,
                ctx.invoke_func_idx,
            );
            code_section_pending = false;
        }

        match payload {
            wasmparser::Payload::TypeSection(reader) => {
                emit_type_section(&mut module, reader, ctx)?;
            },
            wasmparser::Payload::FunctionSection(reader) => {
                emit_function_section(&mut module, reader, ctx)?;
            },
            wasmparser::Payload::ExportSection(reader) => {
                emit_export_section(&mut module, reader, ctx)?;
            },
            wasmparser::Payload::CodeSectionStart { .. } => {
                code_section_pending = true;
                code_bodies.clear();
            },
            wasmparser::Payload::CodeSectionEntry(body) => {
                code_bodies.push(ctx.wasm_bytes[body.range()].to_vec());
            },
            // Copy all other known sections as raw bytes
            ref p => {
                copy_raw_section(&mut module, p, ctx.wasm_bytes);
            },
        }
    }

    if code_section_pending {
        emit_code_section(
            &mut module,
            &code_bodies,
            ctx.export_names,
            ctx.arg_start_func_idx,
            ctx.invoke_func_idx,
        );
    }

    Ok(module.finish())
}

/// Emit the type section, optionally appending a `() -> i32` type.
fn emit_type_section(
    module: &mut wasm_encoder::Module,
    reader: wasmparser::TypeSectionReader<'_>,
    ctx: &StitchContext<'_>,
) -> BridgeResult<()> {
    if ctx.needs_new_type {
        let mut types = wasm_encoder::TypeSection::new();
        for recgroup in reader {
            let recgroup = recgroup.map_err(|e| {
                BridgeError::ExportStitchFailed(format!("failed to read type: {e}"))
            })?;
            reencode_rec_group(&mut types, &recgroup)?;
        }
        types.ty().function([], [wasm_encoder::ValType::I32]);
        module.section(&types);
    } else {
        let range = reader.range();
        module.section(&wasm_encoder::RawSection {
            id: 1,
            data: &ctx.wasm_bytes[range],
        });
    }
    Ok(())
}

/// Emit the function section with wrapper function entries appended.
fn emit_function_section(
    module: &mut wasm_encoder::Module,
    reader: wasmparser::FunctionSectionReader<'_>,
    ctx: &StitchContext<'_>,
) -> BridgeResult<()> {
    let mut funcs = wasm_encoder::FunctionSection::new();
    for type_index in reader {
        let type_index = type_index.map_err(|e| {
            BridgeError::ExportStitchFailed(format!("failed to read function: {e}"))
        })?;
        funcs.function(type_index);
    }
    for _ in ctx.export_names {
        funcs.function(ctx.wrapper_type_idx);
    }
    module.section(&funcs);
    Ok(())
}

/// Emit the export section with named exports appended.
fn emit_export_section(
    module: &mut wasm_encoder::Module,
    reader: wasmparser::ExportSectionReader<'_>,
    ctx: &StitchContext<'_>,
) -> BridgeResult<()> {
    let mut exports = wasm_encoder::ExportSection::new();
    for export in reader {
        let export = export
            .map_err(|e| BridgeError::ExportStitchFailed(format!("failed to read export: {e}")))?;
        exports.export(export.name, convert_export_kind(export.kind), export.index);
    }
    #[expect(clippy::cast_possible_truncation)]
    for (i, name) in ctx.export_names.iter().enumerate() {
        exports.export(
            name,
            wasm_encoder::ExportKind::Func,
            ctx.total_existing_funcs
                .checked_add(i as u32)
                .ok_or_else(|| {
                    BridgeError::ExportStitchFailed("function index overflow".to_string())
                })?,
        );
    }
    module.section(&exports);
    Ok(())
}

/// Copy a section as raw bytes if it's a known passthrough section.
fn copy_raw_section(
    module: &mut wasm_encoder::Module,
    payload: &wasmparser::Payload<'_>,
    wasm_bytes: &[u8],
) {
    let (id, range) = match payload {
        wasmparser::Payload::ImportSection(r) => (2, r.range()),
        wasmparser::Payload::TableSection(r) => (4, r.range()),
        wasmparser::Payload::MemorySection(r) => (5, r.range()),
        wasmparser::Payload::GlobalSection(r) => (6, r.range()),
        wasmparser::Payload::StartSection { range, .. } => (8, range.clone()),
        wasmparser::Payload::ElementSection(r) => (9, r.range()),
        wasmparser::Payload::DataSection(r) => (11, r.range()),
        wasmparser::Payload::DataCountSection { range, .. } => (12, range.clone()),
        wasmparser::Payload::CustomSection(r) => (0, r.range()),
        // Version, End, and unknown payloads are skipped
        _ => return,
    };
    module.section(&wasm_encoder::RawSection {
        id,
        data: &wasm_bytes[range],
    });
}

/// Information gathered from the first parse of the module.
struct ModuleInfo {
    invoke_i32_func_idx: Option<u32>,
    arg_start_func_idx: Option<u32>,
    void_to_i32_type_idx: Option<u32>,
    num_types: u32,
    num_imported_funcs: u32,
    num_defined_funcs: u32,
}

fn gather_module_info(wasm_bytes: &[u8]) -> BridgeResult<ModuleInfo> {
    let mut info = ModuleInfo {
        invoke_i32_func_idx: None,
        arg_start_func_idx: None,
        void_to_i32_type_idx: None,
        num_types: 0,
        num_imported_funcs: 0,
        num_defined_funcs: 0,
    };

    for payload in wasmparser::Parser::new(0).parse_all(wasm_bytes) {
        let payload = payload
            .map_err(|e| BridgeError::ExportStitchFailed(format!("failed to parse WASM: {e}")))?;

        match payload {
            wasmparser::Payload::TypeSection(reader) => {
                #[expect(clippy::cast_possible_truncation)]
                for (i, recgroup) in reader.into_iter().enumerate() {
                    let recgroup = recgroup.map_err(|e| {
                        BridgeError::ExportStitchFailed(format!("failed to read type: {e}"))
                    })?;
                    if is_void_to_i32(&recgroup) {
                        info.void_to_i32_type_idx = Some(i as u32);
                    }
                    info.num_types = i.checked_add(1).ok_or_else(|| {
                        BridgeError::ExportStitchFailed("type count overflow".to_string())
                    })? as u32;
                }
            },
            wasmparser::Payload::ImportSection(reader) => {
                for import in reader {
                    let import = import.map_err(|e| {
                        BridgeError::ExportStitchFailed(format!("failed to read import: {e}"))
                    })?;
                    if matches!(import.ty, wasmparser::TypeRef::Func(_)) {
                        info.num_imported_funcs =
                            info.num_imported_funcs.checked_add(1).ok_or_else(|| {
                                BridgeError::ExportStitchFailed(
                                    "imported function count overflow".to_string(),
                                )
                            })?;
                    }
                }
            },
            wasmparser::Payload::FunctionSection(reader) => {
                info.num_defined_funcs = reader.count();
            },
            wasmparser::Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.map_err(|e| {
                        BridgeError::ExportStitchFailed(format!("failed to read export: {e}"))
                    })?;
                    if export.kind == wasmparser::ExternalKind::Func {
                        match export.name {
                            "__invoke_i32" => {
                                info.invoke_i32_func_idx = Some(export.index);
                            },
                            "__arg_start" => {
                                info.arg_start_func_idx = Some(export.index);
                            },
                            _ => {},
                        }
                    }
                }
            },
            _ => {},
        }
    }

    Ok(info)
}

/// Check if a rec group contains a single function type `() -> i32`.
fn is_void_to_i32(recgroup: &wasmparser::RecGroup) -> bool {
    let types: Vec<_> = recgroup.types().collect();
    if types.len() != 1 {
        return false;
    }
    let sub_ty = &types[0];
    if let wasmparser::CompositeInnerType::Func(func_type) = &sub_ty.composite_type.inner {
        func_type.params().is_empty()
            && func_type.results().len() == 1
            && func_type.results()[0] == wasmparser::ValType::I32
    } else {
        false
    }
}

/// Re-encode a wasmparser `RecGroup` into a wasm-encoder `TypeSection`.
fn reencode_rec_group(
    types: &mut wasm_encoder::TypeSection,
    recgroup: &wasmparser::RecGroup,
) -> BridgeResult<()> {
    let subtypes: Vec<_> = recgroup.types().collect();
    if subtypes.len() != 1 {
        return Err(BridgeError::ExportStitchFailed(
            "multi-type rec groups are not supported".into(),
        ));
    }
    let sub_ty = &subtypes[0];
    match &sub_ty.composite_type.inner {
        wasmparser::CompositeInnerType::Func(func_type) => {
            let params: Vec<wasm_encoder::ValType> = func_type
                .params()
                .iter()
                .map(|v| convert_val_type(*v))
                .collect();
            let results: Vec<wasm_encoder::ValType> = func_type
                .results()
                .iter()
                .map(|v| convert_val_type(*v))
                .collect();
            types.ty().function(params, results);
        },
        _ => {
            return Err(BridgeError::ExportStitchFailed(
                "unsupported composite type (expected function type)".into(),
            ));
        },
    }
    Ok(())
}

/// Convert a wasmparser value type to wasm-encoder value type.
///
/// Reference types are simplified to FUNCREF or EXTERNREF. This is sufficient
/// for the `QuickJS` kernel which only uses basic WASI types. If future kernels
/// use typed function references or GC proposal types, this will need updating.
fn convert_val_type(vt: wasmparser::ValType) -> wasm_encoder::ValType {
    match vt {
        wasmparser::ValType::I32 => wasm_encoder::ValType::I32,
        wasmparser::ValType::I64 => wasm_encoder::ValType::I64,
        wasmparser::ValType::F32 => wasm_encoder::ValType::F32,
        wasmparser::ValType::F64 => wasm_encoder::ValType::F64,
        wasmparser::ValType::V128 => wasm_encoder::ValType::V128,
        wasmparser::ValType::Ref(r) => {
            if r.is_func_ref() {
                wasm_encoder::ValType::FUNCREF
            } else {
                wasm_encoder::ValType::EXTERNREF
            }
        },
    }
}

fn convert_export_kind(kind: wasmparser::ExternalKind) -> wasm_encoder::ExportKind {
    match kind {
        wasmparser::ExternalKind::Func => wasm_encoder::ExportKind::Func,
        wasmparser::ExternalKind::Table => wasm_encoder::ExportKind::Table,
        wasmparser::ExternalKind::Memory => wasm_encoder::ExportKind::Memory,
        wasmparser::ExternalKind::Global => wasm_encoder::ExportKind::Global,
        wasmparser::ExternalKind::Tag => wasm_encoder::ExportKind::Tag,
    }
}

/// Emit the code section with existing bodies plus new wrapper function bodies.
///
/// Each wrapper function calls `__arg_start()` to initialize the argument stack,
/// then `__invoke_i32(index)` to invoke the JS function at the given sorted index.
/// The `QuickJS` kernel requires `__arg_start()` before every `__invoke_*()` call
/// to push an empty argument vector onto the `CALL_ARGS` stack.
fn emit_code_section(
    module: &mut wasm_encoder::Module,
    existing_bodies: &[Vec<u8>],
    export_names: &[&str],
    arg_start_func_idx: u32,
    invoke_func_idx: u32,
) {
    let mut code = wasm_encoder::CodeSection::new();
    for body_bytes in existing_bodies {
        code.raw(body_bytes);
    }
    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    for (i, _name) in export_names.iter().enumerate() {
        let mut f = wasm_encoder::Function::new([]);
        f.instruction(&wasm_encoder::Instruction::Call(arg_start_func_idx));
        f.instruction(&wasm_encoder::Instruction::I32Const(i as i32));
        f.instruction(&wasm_encoder::Instruction::Call(invoke_func_idx));
        f.instruction(&wasm_encoder::Instruction::End);
        code.function(&f);
    }
    module.section(&code);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal WASM module with `__arg_start` and `__invoke_i32` exports for testing.
    fn build_test_module() -> Vec<u8> {
        let mut module = wasm_encoder::Module::new();

        let mut types = wasm_encoder::TypeSection::new();
        // type 0: (i32) -> i32  — for __invoke_i32
        types
            .ty()
            .function([wasm_encoder::ValType::I32], [wasm_encoder::ValType::I32]);
        // type 1: () -> i32  — for wrapper exports
        types.ty().function([], [wasm_encoder::ValType::I32]);
        // type 2: () -> ()  — for __arg_start
        types.ty().function([], []);
        module.section(&types);

        let mut functions = wasm_encoder::FunctionSection::new();
        functions.function(0); // func 0: __invoke_i32
        functions.function(2); // func 1: __arg_start
        module.section(&functions);

        let mut exports = wasm_encoder::ExportSection::new();
        exports.export("__invoke_i32", wasm_encoder::ExportKind::Func, 0);
        exports.export("__arg_start", wasm_encoder::ExportKind::Func, 1);
        module.section(&exports);

        let mut code = wasm_encoder::CodeSection::new();
        // func 0: __invoke_i32 — returns its argument
        let mut f = wasm_encoder::Function::new([]);
        f.instruction(&wasm_encoder::Instruction::LocalGet(0));
        f.instruction(&wasm_encoder::Instruction::End);
        code.function(&f);
        // func 1: __arg_start — no-op
        let mut f = wasm_encoder::Function::new([]);
        f.instruction(&wasm_encoder::Instruction::End);
        code.function(&f);
        module.section(&code);

        module.finish()
    }

    #[test]
    fn stitch_adds_named_exports() {
        let wasm = build_test_module();
        let result =
            stitch_exports(&wasm, &["describe-tools", "execute-tool", "run-hook"]).unwrap();

        assert_eq!(&result[..4], b"\0asm");

        let mut found_exports = Vec::new();
        for payload in wasmparser::Parser::new(0).parse_all(&result) {
            if let wasmparser::Payload::ExportSection(reader) = payload.unwrap() {
                for export in reader {
                    found_exports.push(export.unwrap().name.to_string());
                }
            }
        }

        for name in ["__invoke_i32", "describe-tools", "execute-tool", "run-hook"] {
            assert!(
                found_exports.contains(&name.to_string()),
                "should have export {name}"
            );
        }
    }

    #[test]
    fn stitch_output_is_valid_wasm() {
        let wasm = build_test_module();
        let result =
            stitch_exports(&wasm, &["describe-tools", "execute-tool", "run-hook"]).unwrap();

        let mut section_count = 0;
        for payload in wasmparser::Parser::new(0).parse_all(&result) {
            payload.expect("output WASM should parse cleanly");
            section_count += 1;
        }
        assert!(section_count > 0);
    }

    #[test]
    fn stitch_fails_without_invoke() {
        let mut module = wasm_encoder::Module::new();
        let mut types = wasm_encoder::TypeSection::new();
        types.ty().function([], []);
        module.section(&types);
        let wasm = module.finish();

        let err = stitch_exports(&wasm, &["test"]).unwrap_err();
        assert!(
            err.to_string().contains("__invoke_i32"),
            "error should mention __invoke_i32, got: {err}"
        );
    }
}
