//! Bundled daemon binary — installed alongside `astrid` via `cargo install astrid`.
//!
//! Delegates to the shared `astrid_daemon::run()` library function. This is
//! identical to the standalone `astrid-daemon` binary but co-installed with
//! the CLI so `find_companion_binary("astrid-daemon")` always finds it.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    astrid_daemon::run().await
}
