//! `astrid capsule list` - display all installed capsules with interface metadata.

use std::collections::HashMap;

use colored::Colorize;

use super::meta::scan_installed_capsules;
use crate::theme::Theme;

/// List all installed capsules with their provides/requires metadata.
///
/// In default mode, shows a compact one-line-per-capsule view with capability
/// counts. With `--verbose`, expands each capsule to show the full capability
/// list and install source.
pub(crate) fn list_capsules(verbose: bool) -> anyhow::Result<()> {
    let capsules = scan_installed_capsules()?;

    if capsules.is_empty() {
        println!("{}", Theme::info("No capsules installed."));
        return Ok(());
    }

    println!(
        "{} ({})",
        Theme::header("Installed Capsules"),
        capsules.len()
    );
    println!("{}", Theme::separator());

    if verbose {
        print_verbose(&capsules);
    } else {
        print_compact(&capsules);
    }

    println!(
        "\n{} capsule(s) installed",
        capsules.len().to_string().bold()
    );
    Ok(())
}

/// Compact: one line per capsule.
fn print_compact(capsules: &[super::meta::InstalledCapsule]) {
    let max_name_len = capsules.iter().map(|c| c.name.len()).max().unwrap_or(30);
    let max_version_len = capsules
        .iter()
        .map(|c| c.meta.as_ref().map_or(7, |m| m.version.len()))
        .max()
        .unwrap_or(7); // "unknown".len()

    for cap in capsules {
        let (version, exports_count, imports_count) = match &cap.meta {
            Some(meta) => (
                meta.version.as_str(),
                meta.exports.values().map(HashMap::len).sum::<usize>(),
                meta.imports.values().map(HashMap::len).sum::<usize>(),
            ),
            None => ("unknown", 0, 0),
        };

        let location_tag = format!("[{}]", cap.location);
        let caps_summary = format!("exports: {exports_count}, imports: {imports_count}");

        // Pad the name before applying bold to avoid ANSI escape codes
        // distorting the column width calculation.
        let padded_name = format!("{:<width$}", cap.name, width = max_name_len);
        println!(
            "  {} {:<width$} {:<13} {}",
            padded_name.bold(),
            version,
            Theme::dimmed(&location_tag),
            Theme::dimmed(&caps_summary),
            width = max_version_len,
        );
    }
}

/// Verbose: full details per capsule.
fn print_verbose(capsules: &[super::meta::InstalledCapsule]) {
    for (i, cap) in capsules.iter().enumerate() {
        if i > 0 {
            println!();
        }

        let Some(meta) = &cap.meta else {
            let version = "unknown";
            println!(
                "{}  {}  {}",
                cap.name.bold(),
                version,
                Theme::dimmed(&format!("[{}]", cap.location)),
            );
            println!("  {}", Theme::dimmed("(no metadata)"));
            continue;
        };
        let (version, source) = (meta.version.as_str(), meta.source.as_deref());

        println!(
            "{}  {}  {}",
            cap.name.bold(),
            version,
            Theme::dimmed(&format!("[{}]", cap.location)),
        );

        if let Some(src) = source {
            println!("  {}", Theme::kv("Source", src));
        }

        print_interface_map("Exports", &meta.exports);
        print_interface_map("Imports", &meta.imports);
        print_topics(&meta.topics);
    }
}

/// Print a labelled interface map (imports or exports), or "(none)" if empty.
fn print_interface_map(
    label: &str,
    map: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
) {
    if map.is_empty() {
        println!("  {}: {}", label.bold(), Theme::dimmed("(none)"));
    } else {
        println!("  {}:", label.bold());
        for (ns, ifaces) in map {
            for (name, version) in ifaces {
                println!("    {ns}/{name} {version}");
            }
        }
    }
}

/// Maximum number of schema lines to display before truncating.
const MAX_SCHEMA_DISPLAY_LINES: usize = 20;

/// Print topic API declarations, if any.
fn print_topics(topics: &[super::meta::BakedTopic]) {
    if topics.is_empty() {
        return;
    }
    println!("  {}:", "Topics".bold());
    for topic in topics {
        let desc = topic.description.as_deref().unwrap_or_default();
        let desc_suffix = if desc.is_empty() {
            String::new()
        } else {
            format!(" - {desc}")
        };
        println!(
            "    {} {}{}",
            topic.name,
            Theme::dimmed(&format!("[{}]", topic.direction)),
            Theme::dimmed(&desc_suffix),
        );
        if let Some(ref schema) = topic.schema {
            let pretty = serde_json::to_string_pretty(schema)
                .unwrap_or_else(|e| format!("<schema serialization error: {e}>"));
            let lines: Vec<&str> = pretty.lines().collect();
            if lines.len() > MAX_SCHEMA_DISPLAY_LINES {
                for line in &lines[..MAX_SCHEMA_DISPLAY_LINES] {
                    println!("      {line}");
                }
                let remaining = lines.len().saturating_sub(MAX_SCHEMA_DISPLAY_LINES);
                println!(
                    "      {}",
                    Theme::dimmed(&format!("... ({remaining} more lines)"))
                );
            } else {
                for line in &lines {
                    println!("      {line}");
                }
            }
        }
    }
}
