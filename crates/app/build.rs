fn main() {
    slint_build::compile_with_config("ui/panel.slint", slint_build::CompilerConfiguration::new()).expect("ui/panel.slint");
}
