use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let object = out_dir.join("libm_extra.o");
    let archive = out_dir.join("libtumulus_extra.a");
    let status = Command::new("cc")
        .args(["-c", "-O2", "src/libm_extra.c", "-o"])
        .arg(&object)
        .status()
        .expect("cc");
    assert!(status.success(), "cc failed to compile src/libm_extra.c");
    let status = Command::new("ar")
        .args(["crs"])
        .arg(&archive)
        .arg(&object)
        .status()
        .expect("ar");
    assert!(status.success(), "ar failed to archive libm_extra.o");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=tumulus_extra");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rerun-if-changed=src/libm_extra.c");
}
