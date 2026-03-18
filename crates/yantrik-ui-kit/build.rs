fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let slint_path = format!("{}/slint", manifest_dir);
    // Expose slint directory path to dependent crates via DEP_YANTRIK_UI_KIT_SLINT_PATH
    println!("cargo:slint_path={slint_path}");
    println!("cargo:rerun-if-changed=slint/");
}
