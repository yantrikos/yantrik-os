fn main() {
    // The shared theme (colours, Barlow), so the session lock looks like the lock screen it replaces.
    let tokens = std::env::var("DEP_YANTRIK_DESIGN_TOKENS_SLINT_PATH")
        .expect("yantrik-design-tokens provides DEP_YANTRIK_DESIGN_TOKENS_SLINT_PATH");
    let config = slint_build::CompilerConfiguration::new().with_include_paths(vec![tokens.into()]);
    slint_build::compile_with_config("ui/lock.slint", config).unwrap();
}
