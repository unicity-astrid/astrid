//! Keys command — manage cryptographic identity.

use astralis_core::dirs::AstralisHome;
use astralis_crypto::KeyPair;

use crate::theme::Theme;

/// Show the current key (public key hex and key ID).
pub(crate) fn show_key() -> anyhow::Result<()> {
    let home = AstralisHome::resolve()?;
    home.ensure()?;

    let key_path = home.user_key_path();
    if !key_path.exists() {
        println!("{}", Theme::info("No key found. Generating one..."));
    }

    let key = KeyPair::load_or_generate(key_path)?;

    println!("\n{}", Theme::header("Cryptographic Identity"));
    println!("  Key ID:     {}", key.key_id_hex());
    println!("  Public key: {}", hex::encode(key.public_key_bytes()));
    println!("  Key file:   {}", home.user_key_path().display());
    println!();

    Ok(())
}

/// Generate a new key, with confirmation if one already exists.
pub(crate) fn generate_key(force: bool) -> anyhow::Result<()> {
    let home = AstralisHome::resolve()?;
    home.ensure()?;

    let key_path = home.user_key_path();

    if key_path.exists() && !force {
        println!(
            "{}",
            Theme::warning("A key already exists. This will replace it.")
        );
        println!(
            "{}",
            Theme::warning("Existing audit chains and capability tokens will become unverifiable.")
        );
        println!();

        let confirm = dialoguer::Confirm::new()
            .with_prompt("Replace existing key?")
            .default(false)
            .interact()?;

        if !confirm {
            println!("{}", Theme::info("Aborted."));
            return Ok(());
        }

        // Remove existing key so load_or_generate creates a new one.
        std::fs::remove_file(&key_path)?;
    }

    let key = KeyPair::load_or_generate(key_path)?;

    println!("{}", Theme::success("New key generated."));
    println!("  Key ID:     {}", key.key_id_hex());
    println!("  Public key: {}", hex::encode(key.public_key_bytes()));
    println!("  Key file:   {}", home.user_key_path().display());
    println!();

    Ok(())
}
