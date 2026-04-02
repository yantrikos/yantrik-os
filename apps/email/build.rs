fn main() {
    let design_tokens_path = std::env::var("DEP_YANTRIK_DESIGN_TOKENS_SLINT_PATH")
        .expect("yantrik-design-tokens must be a dependency");
    let ui_kit_path = std::env::var("DEP_YANTRIK_UI_KIT_SLINT_PATH")
        .expect("yantrik-ui-kit must be a dependency");

    // Include the shared UI directory so we can import email.slint, theme.slint, etc.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let shared_ui_path = std::path::Path::new(&manifest_dir)
        .join("../../crates/yantrik-ui-slint/ui");

    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        .with_include_paths(vec![
            design_tokens_path.into(),
            ui_kit_path.into(),
            shared_ui_path,
        ]);

    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
