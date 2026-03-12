//! `astrid capsule list` - display all installed capsules with capability metadata.

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
    for cap in capsules {
        let (version, provides_count, requires_count) = match &cap.meta {
            Some(meta) => (
                meta.version.as_str(),
                meta.provides.len(),
                meta.requires.len(),
            ),
            None => ("unknown", 0, 0),
        };

        let location_tag = format!("[{}]", cap.location);
        let caps_summary = format!("provides: {provides_count}, requires: {requires_count}");

        println!(
            "  {:<30} {:<8} {:<13} {}",
            cap.name.bold(),
            version,
            Theme::dimmed(&location_tag),
            Theme::dimmed(&caps_summary),
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
        let (version, source, provides, requires) = (
            meta.version.as_str(),
            meta.source.as_deref(),
            meta.provides.as_slice(),
            meta.requires.as_slice(),
        );

        println!(
            "{}  {}  {}",
            cap.name.bold(),
            version,
            Theme::dimmed(&format!("[{}]", cap.location)),
        );

        if let Some(src) = source {
            println!("  {}", Theme::kv("Source", src));
        }

        print_capability_list("Provides", provides);
        print_capability_list("Requires", requires);
    }
}

/// Print a labelled capability list, or "(none)" if empty.
fn print_capability_list(label: &str, caps: &[String]) {
    if caps.is_empty() {
        println!("  {}: {}", label.bold(), Theme::dimmed("(none)"));
    } else {
        println!("  {}:", label.bold());
        for cap in caps {
            println!("    {cap}");
        }
    }
}
