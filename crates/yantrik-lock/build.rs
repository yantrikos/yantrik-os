fn main() {
    // The shared theme (colours, Barlow) and the kit's lock view, so the session lock is drawn by the
    // same component as the shell's own lock screen.
    let tokens = std::env::var("DEP_YANTRIK_DESIGN_TOKENS_SLINT_PATH")
        .expect("yantrik-design-tokens provides DEP_YANTRIK_DESIGN_TOKENS_SLINT_PATH");
    let kit = std::env::var("DEP_YANTRIK_UI_KIT_SLINT_PATH")
        .expect("yantrik-ui-kit provides DEP_YANTRIK_UI_KIT_SLINT_PATH");
    let config = slint_build::CompilerConfiguration::new().with_include_paths(vec![tokens.into(), kit.into()]);
    slint_build::compile_with_config("ui/lock.slint", config).unwrap();
}
