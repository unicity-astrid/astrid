//! OXC-based `TypeScript`/`JavaScript` transpilation.
//!
//! Replaces the esbuild-based bundler with a pure-Rust pipeline:
//! 1. Parse with OXC (TS or JS based on filename extension)
//! 2. Reject non-type import declarations (single-file plugins only)
//! 3. Strip `TypeScript` types via `oxc_transformer`
//! 4. Generate JS with `oxc_codegen`
//! 5. Post-process ESM → CJS (`export default`, `export function`, `export const`)

use std::path::Path;

use oxc::codegen::Codegen;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{TransformOptions, Transformer};

use crate::error::{BridgeError, BridgeResult};

/// Transpile a JS or TS source string to CJS-compatible `JavaScript`.
///
/// `filename` is used to determine the source type (`.ts`, `.tsx`, `.js`, `.jsx`).
/// Returns the transpiled `JavaScript` source code.
///
/// # Errors
///
/// - `BridgeError::TranspileFailed` if parsing or transformation fails
/// - `BridgeError::UnresolvedImports` if the source contains non-type import declarations
pub fn transpile(source: &str, filename: &str) -> BridgeResult<String> {
    let allocator = oxc_allocator::Allocator::default();

    let source_type = SourceType::from_path(filename).unwrap_or_else(|_| {
        // Default to JS module if extension is unrecognized
        SourceType::mjs()
    });

    // 1. Parse
    let parse_ret = Parser::new(&allocator, source, source_type).parse();

    if parse_ret.panicked {
        let errors: Vec<String> = parse_ret.errors.iter().map(|e| format!("{e}")).collect();
        return Err(BridgeError::TranspileFailed(format!(
            "parse errors:\n{}",
            errors.join("\n")
        )));
    }

    if !parse_ret.errors.is_empty() {
        let errors: Vec<String> = parse_ret.errors.iter().map(|e| format!("{e}")).collect();
        return Err(BridgeError::TranspileFailed(format!(
            "parse errors:\n{}",
            errors.join("\n")
        )));
    }

    let mut program = parse_ret.program;

    // 2. Check for non-type imports
    check_imports(&program)?;

    // 3. Semantic analysis (required for transformer)
    let sem_ret = SemanticBuilder::new()
        .with_excess_capacity(2.0)
        .build(&program);

    let scoping = sem_ret.semantic.into_scoping();

    // 4. Transform (strip TypeScript types)
    let transform_options = TransformOptions::default();
    let path = Path::new(filename);
    let transform_ret = Transformer::new(&allocator, path, &transform_options)
        .build_with_scoping(scoping, &mut program);

    if !transform_ret.errors.is_empty() {
        let errors: Vec<String> = transform_ret
            .errors
            .iter()
            .map(|e| format!("{e}"))
            .collect();
        return Err(BridgeError::TranspileFailed(format!(
            "transform errors:\n{}",
            errors.join("\n")
        )));
    }

    // 5. Code generation
    let codegen_ret = Codegen::new().build(&program);
    let js_output = codegen_ret.code;

    // 6. ESM → CJS post-processing
    Ok(esm_to_cjs(&js_output))
}

/// Check for non-type import declarations.
///
/// Single-file plugins should not have runtime imports. Type-only imports
/// (`import type { ... }`) are allowed since they're erased by the transformer.
fn check_imports(program: &oxc::ast::ast::Program) -> BridgeResult<()> {
    let mut bad_imports = Vec::new();

    for stmt in &program.body {
        if let oxc::ast::ast::Statement::ImportDeclaration(decl) = stmt {
            // Allow type-only imports (they get stripped)
            if decl.import_kind.is_type() {
                continue;
            }
            bad_imports.push(decl.source.value.to_string());
        }
    }

    if !bad_imports.is_empty() {
        let modules = bad_imports.join(", ");
        return Err(BridgeError::UnresolvedImports(format!(
            "plugin imports modules that cannot be resolved at runtime: [{modules}]. \
             Single-file plugins must be self-contained. If your plugin needs external \
             dependencies, pre-bundle it with esbuild or rollup before running openclaw-bridge."
        )));
    }

    Ok(())
}

/// Post-process OXC codegen output to convert ESM exports to CJS.
///
/// Handles the narrow set of patterns used in single-file plugins:
/// - `export default X` → `module.exports = X`
/// - `export function name(` → `function name(` + `module.exports.name = name;`
/// - `export const name =` → `const name =` + `module.exports.name = name;`
/// - `export { name }` and `export { name as alias }` → `module.exports.name = name;`
fn esm_to_cjs(js: &str) -> String {
    let mut output_lines = Vec::new();
    let mut deferred_exports: Vec<String> = Vec::new();

    for line in js.lines() {
        let trimmed = line.trim();

        // export default <expr>
        if let Some(rest) = trimmed.strip_prefix("export default ") {
            output_lines.push(format!("module.exports = {rest}"));
            continue;
        }

        // export function name(...)
        if let Some(rest) = trimmed.strip_prefix("export function ") {
            // Extract function name (everything before the first `(`)
            if let Some(paren_idx) = rest.find('(') {
                let name = rest[..paren_idx].trim();
                // Handle `async function` — the name is after `async `
                output_lines.push(format!("function {rest}"));
                deferred_exports.push(format!("module.exports.{name} = {name};"));
            } else {
                output_lines.push(line.to_string());
            }
            continue;
        }

        // export async function name(...)
        if let Some(rest) = trimmed.strip_prefix("export async function ") {
            if let Some(paren_idx) = rest.find('(') {
                let name = rest[..paren_idx].trim();
                output_lines.push(format!("async function {rest}"));
                deferred_exports.push(format!("module.exports.{name} = {name};"));
            } else {
                output_lines.push(line.to_string());
            }
            continue;
        }

        // export const name = ... / export let name = ... / export var name = ...
        if let Some(rest) = trimmed
            .strip_prefix("export const ")
            .or_else(|| trimmed.strip_prefix("export let "))
            .or_else(|| trimmed.strip_prefix("export var "))
        {
            let keyword = if trimmed.starts_with("export const") {
                "const"
            } else if trimmed.starts_with("export let") {
                "let"
            } else {
                "var"
            };

            // Extract the variable name (before `=` or `:` for TS type annotations)
            if let Some(eq_idx) = rest.find('=') {
                let name = rest[..eq_idx].trim().trim_end_matches(':').trim();
                // Strip any type annotation
                let name = name.split(':').next().unwrap_or(name).trim();
                output_lines.push(format!("{keyword} {rest}"));
                deferred_exports.push(format!("module.exports.{name} = {name};"));
            } else {
                output_lines.push(format!("{keyword} {rest}"));
            }
            continue;
        }

        // export { name } or export { name as alias, ... }
        if let Some(rest) = trimmed.strip_prefix("export {") {
            if let Some(brace_end) = rest.find('}') {
                let specifiers = &rest[..brace_end];
                for spec in specifiers.split(',') {
                    let spec = spec.trim();
                    if spec.is_empty() {
                        continue;
                    }
                    if let Some((local, exported)) =
                        spec.split_once(" as ").map(|(l, e)| (l.trim(), e.trim()))
                    {
                        deferred_exports.push(format!("module.exports.{exported} = {local};"));
                    } else {
                        deferred_exports.push(format!("module.exports.{spec} = {spec};"));
                    }
                }
            }
            // Don't emit the `export { ... }` line itself
            continue;
        }

        // export class name ...
        if let Some(rest) = trimmed.strip_prefix("export class ") {
            if let Some(name_end) = rest.find([' ', '{']) {
                let name = rest[..name_end].trim();
                output_lines.push(format!("class {rest}"));
                deferred_exports.push(format!("module.exports.{name} = {name};"));
            } else {
                output_lines.push(line.to_string());
            }
            continue;
        }

        // No transformation needed
        output_lines.push(line.to_string());
    }

    // Append deferred exports at the end
    if !deferred_exports.is_empty() {
        output_lines.push(String::new());
        output_lines.extend(deferred_exports);
    }

    output_lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transpile_plain_js_passthrough() {
        let source = "const x = 42;\nconsole.log(x);\n";
        let result = transpile(source, "plugin.js").unwrap();
        assert!(result.contains("const x = 42"));
        assert!(result.contains("console.log(x)"));
    }

    #[test]
    fn transpile_ts_strips_types() {
        let source = r#"
const greet = (name: string): string => {
    return "hello " + name;
};
"#;
        let result = transpile(source, "plugin.ts").unwrap();
        assert!(result.contains("const greet ="));
        // Type annotations should be stripped
        assert!(!result.contains(": string"));
    }

    #[test]
    fn transpile_ts_interface_stripped() {
        let source = r#"
interface Config {
    apiKey: string;
    timeout: number;
}
const cfg: Config = { apiKey: "test", timeout: 30 };
"#;
        let result = transpile(source, "plugin.ts").unwrap();
        assert!(!result.contains("interface Config"));
        assert!(result.contains("const cfg"));
    }

    #[test]
    fn transpile_rejects_runtime_imports() {
        let source = r#"
import { readFile } from "fs";
console.log(readFile);
"#;
        let err = transpile(source, "plugin.js").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unresolved imports"), "got: {msg}");
        assert!(msg.contains("fs"), "got: {msg}");
    }

    #[test]
    fn transpile_allows_type_imports() {
        let source = r#"
import type { Config } from "./types";
const x: number = 42;
"#;
        // Type imports should be allowed (they're erased)
        let result = transpile(source, "plugin.ts").unwrap();
        assert!(result.contains("42"));
        // The import type should be stripped
        assert!(!result.contains("import type"));
    }

    #[test]
    fn esm_to_cjs_export_default() {
        let input = "export default function activate(ctx) {\n  return ctx;\n}\n";
        let output = esm_to_cjs(input);
        assert!(
            output.contains("module.exports = function activate(ctx)"),
            "got: {output}"
        );
    }

    #[test]
    fn esm_to_cjs_export_function() {
        let input = "export function greet(name) {\n  return name;\n}\n";
        let output = esm_to_cjs(input);
        assert!(
            output.contains("function greet(name)"),
            "should strip export keyword, got: {output}"
        );
        assert!(
            output.contains("module.exports.greet = greet;"),
            "should add CJS export, got: {output}"
        );
    }

    #[test]
    fn esm_to_cjs_export_const() {
        let input = "export const VERSION = \"1.0.0\";\n";
        let output = esm_to_cjs(input);
        assert!(
            output.contains("const VERSION = \"1.0.0\""),
            "got: {output}"
        );
        assert!(
            output.contains("module.exports.VERSION = VERSION;"),
            "got: {output}"
        );
    }

    #[test]
    fn esm_to_cjs_no_exports_passthrough() {
        let input = "const x = 42;\nconsole.log(x);\n";
        let output = esm_to_cjs(input);
        assert_eq!(output.trim(), input.trim());
    }
}
