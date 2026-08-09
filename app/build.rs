fn main() {
    let ui_file =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ui/main.slint");
    slint_build::compile(ui_file).expect("failed to compile Slint UI");
}
