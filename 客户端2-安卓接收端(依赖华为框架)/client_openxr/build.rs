use std::{env, path::PathBuf};

fn main() {
    if env::var_os("CARGO_CFG_TARGET_OS").as_deref() != Some("android".as_ref()) {
        return;
    }

    let abi = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64-v8a",
        _ => "armeabi-v7a",
    };
    let loader_directory =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"))
            .join("../../deps/android_openxr")
            .join(abi);

    println!(
        "cargo:rustc-link-search=native={}",
        loader_directory.display()
    );
    println!("cargo:rerun-if-changed={}", loader_directory.display());
}
