// Licensed under the Apache-2.0 license

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rerun-if-changed=link.ld");
    println!("cargo:rustc-link-arg=-T{dir}/link.ld");
    // The linker writes the flat image that is loaded at address 0.
    println!("cargo:rustc-link-arg=--oformat=binary");
}
