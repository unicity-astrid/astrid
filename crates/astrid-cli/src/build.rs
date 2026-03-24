//! Bundled `astrid-build` binary — installed alongside `astrid` via `cargo install astrid`.

fn main() -> anyhow::Result<()> {
    astrid_build::run()
}
