//! Standalone daemon binary entry point.
//!
//! Delegates to the shared `astrid_daemon::run()` library function.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    astrid_daemon::run().await
}
