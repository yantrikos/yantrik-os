fn main() {
    // Resolve shared design tokens path from yantrik-design-tokens crate (via `links` mechanism)
    let design_tokens_path = std::env::var("DEP_YANTRIK_DESIGN_TOKENS_SLINT_PATH")
        .expect("yantrik-design-tokens must be a dependency (provides DEP_YANTRIK_DESIGN_TOKENS_SLINT_PATH)");

    // Resolve shared UI kit path from yantrik-ui-kit crate
    let ui_kit_path = std::env::var("DEP_YANTRIK_UI_KIT_SLINT_PATH")
        .expect("yantrik-ui-kit must be a dependency (provides DEP_YANTRIK_UI_KIT_SLINT_PATH)");

    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent-dark".into())
        .with_include_paths(vec![design_tokens_path.into(), ui_kit_path.into()]);
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
