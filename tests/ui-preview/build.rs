fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let design = root.join("crates/yantrik-design-tokens/slint");
    let kit = root.join("crates/yantrik-ui-kit/slint");
    // Screens that moved into their app import the tokens and the kit as libraries, the way the
    // app's own build does (frameworks/yantrik-app-build).
    let libraries = std::collections::HashMap::from([
        ("yantrik-design".to_string(), design.clone()),
        ("yantrik-kit".to_string(), kit.clone()),
    ]);
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        .with_include_paths(vec![
            design,
            kit,
            root.join("crates/yantrik-ui-slint/ui"),
            root.join("apps/weather/ui"),
            root.join("apps/system-monitor/ui"),
        ])
        .with_library_paths(libraries);
    slint_build::compile_with_config("preview.slint", config).unwrap();
}
