//! Init command — initialize a workspace.

use astrid_core::dirs::WorkspaceDir;

use crate::theme::Theme;

/// Initialize the current directory as an Astrid workspace.
pub(crate) fn run_init() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = WorkspaceDir::from_path(&cwd);

    if ws.dot_astrid().exists() {
        println!(
            "{}",
            Theme::info(&format!(
                "Workspace already initialized at {}",
                cwd.display()
            ))
        );
        return Ok(());
    }

    ws.ensure()?;

    // Write a template config.toml if one doesn't exist.
    let config_path = ws.dot_astrid().join("config.toml");
    if !config_path.exists() {
        std::fs::write(
            &config_path,
            "# Astrid workspace configuration\n\
             # See docs for available options.\n\
             \n\
             # [model]\n\
             # provider = \"anthropic\"\n\
             # model = \"claude-sonnet-4-5-20250929\"\n\
             \n\
             # [security]\n\
             # auto_approve_read = true\n\
             \n\
             # [budget]\n\
             # session_max_usd = 10.0\n",
        )?;
    }

    println!(
        "{}",
        Theme::success(&format!(
            "Initialized workspace at {}",
            ws.dot_astrid().display()
        ))
    );
    println!("  Created: {}", ws.dot_astrid().display());
    println!("  Config:  {}", config_path.display());
    println!();

    Ok(())
}
