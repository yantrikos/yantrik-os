fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut include = vec![
        root.join("crates/yantrik-design-tokens/slint"),
        root.join("crates/yantrik-ui-kit/slint"),
        root.join("crates/yantrik-ui-slint/ui"),
    ];
    // A directory holding a modified theme.slint (another typeface, other sizes) that the scenes
    // are compiled against instead of the tokens' own, for side-by-side renders without touching
    // the tokens. Unset, the preview draws exactly what the OS draws.
    println!("cargo:rerun-if-env-changed=PREVIEW_THEME_DIR");
    if let Some(dir) = std::env::var_os("PREVIEW_THEME_DIR") {
        include.insert(0, std::path::PathBuf::from(dir));
    }
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        .with_include_paths(include);
    slint_build::compile_with_config("preview.slint", config).unwrap();
}
