use std::path::PathBuf;
use std::process;

use clap::Parser;
use rascii_art::camera::{CameraConfig, CameraSource};

#[derive(Debug, Parser)]
#[command(about = "Capture a camera frame and save as JPEG")]
struct Args {
    /// Output path for the captured frame
    #[arg(short, long, default_value = "/tmp/astrid_frame.jpg")]
    output: PathBuf,

    /// Camera device index
    #[arg(long, default_value_t = 0)]
    index: u32,

    /// Disable selfie mirror
    #[arg(long)]
    no_mirror: bool,

    /// Number of warmup frames for auto-exposure
    #[arg(long, default_value_t = 30)]
    warmup: u32,
}

fn main() {
    let args = Args::parse();

    let config = CameraConfig {
        index: args.index,
        mirror: !args.no_mirror,
    };

    let mut source = match CameraSource::new(&config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("camera error: {e}");
            process::exit(1);
        }
    };

    source.warmup(args.warmup);

    let image = match source.frame() {
        Ok(img) => img,
        Err(e) => {
            eprintln!("capture error: {e}");
            process::exit(1);
        }
    };

    if let Err(e) = image.save(&args.output) {
        eprintln!("save error: {e}");
        process::exit(1);
    }

    // Print the path so callers can read it from stdout
    println!("{}", args.output.display());
}
