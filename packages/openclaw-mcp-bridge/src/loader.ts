/**
 * Plugin loader via jiti v2.
 *
 * Uses jiti (the same loader OpenClaw uses) to dynamically import
 * TypeScript/JavaScript plugin entry points with ESM + top-level
 * await support.
 */

import { createJiti } from "jiti";
import * as path from "node:path";
import * as fs from "node:fs";

import {
  PluginManifest,
  PluginManifestSchema,
  PluginModule,
} from "./types.js";

/** Supported major version of the OpenClaw plugin engine. */
const SUPPORTED_MAJOR = 0;

/**
 * Load and validate the `openclaw.plugin.json` manifest from a
 * plugin directory.
 *
 * @param pluginDir - Absolute path to the plugin directory.
 * @returns Parsed and validated plugin manifest.
 * @throws If the manifest file is missing or invalid.
 */
export function loadManifest(pluginDir: string): PluginManifest {
  const manifestPath = path.join(pluginDir, "openclaw.plugin.json");

  if (!fs.existsSync(manifestPath)) {
    throw new Error(
      `Plugin manifest not found: ${manifestPath}`
    );
  }

  const raw = JSON.parse(fs.readFileSync(manifestPath, "utf-8"));
  const manifest = PluginManifestSchema.parse(raw);

  // Version guard: check engine compatibility
  if (manifest.engines?.openclaw) {
    const engineVersion = manifest.engines.openclaw;
    const majorMatch = engineVersion.match(/^[~^]?(\d+)/);
    if (majorMatch) {
      const major = parseInt(majorMatch[1], 10);
      if (major > SUPPORTED_MAJOR) {
        throw new Error(
          `Plugin '${manifest.id}' requires OpenClaw engine ^${major}.0.0, ` +
            `but this bridge only supports ^${SUPPORTED_MAJOR}.x.x`
        );
      }
    }
  }

  return manifest;
}

/**
 * Dynamically import the plugin module from its entry point.
 *
 * Uses jiti v2 which supports:
 * - TypeScript (ESM and CJS)
 * - JavaScript (ESM and CJS)
 * - Top-level await
 *
 * @param pluginDir - Absolute path to the plugin directory.
 * @param entryPoint - Relative path to the entry point (from manifest `main`).
 * @returns The plugin module's default or named export.
 */
export async function loadPluginModule(
  pluginDir: string,
  entryPoint: string
): Promise<PluginModule> {
  const fullPath = path.resolve(pluginDir, entryPoint);

  if (!fs.existsSync(fullPath)) {
    throw new Error(
      `Plugin entry point not found: ${fullPath}`
    );
  }

  const jiti = createJiti(pluginDir, {
    // Enable ESM interop so both CJS and ESM plugins work
    interopDefault: true,
  });

  const mod = await jiti.import(fullPath);

  // Support both default export and named exports
  const pluginModule = (mod as Record<string, unknown>).default ?? mod;

  // Validate that the module has an `activate` function
  if (
    typeof pluginModule !== "object" ||
    pluginModule === null ||
    typeof (pluginModule as PluginModule).activate !== "function"
  ) {
    throw new Error(
      `Plugin entry point at ${fullPath} must export an object with ` +
        `an 'activate' function. Got: ${typeof pluginModule}`
    );
  }

  return pluginModule as PluginModule;
}
