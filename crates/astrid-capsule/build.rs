fn main() {
    let target = std::env::var("TARGET").expect("TARGET environment variable not set by Cargo");
    println!("cargo:rustc-env=TARGET={target}");
}
