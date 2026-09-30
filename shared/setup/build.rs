//! Build script that exposes the compilation target triple to the crate.
//!
//! Owns one thing: the `SLATE_SETUP_TARGET` compile-time environment variable,
//! which `Env::platform` uses to name the release archives (`<name>-<target>`).
//! The triple carries the musl-versus-glibc distinction that `std` does not.

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo:rustc-env=SLATE_SETUP_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
