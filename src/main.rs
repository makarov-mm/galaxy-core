#[cfg(feature = "render")]
fn main() {
    galaxy::app::run();
}

#[cfg(not(feature = "render"))]
fn main() {
    eprintln!("This binary needs the GL renderer. Build it with:");
    eprintln!("    cargo run --release --features render");
    eprintln!("Core verification (no GL):");
    eprintln!("    cargo run --release --bin verify");
}
