fn main() {
    const CAPABILITY: &str = "../../apps/desktop/src-tauri/capabilities/windows-overlay.json";

    println!("cargo:rerun-if-changed={CAPABILITY}");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .capabilities_path_pattern(CAPABILITY)
            .codegen(tauri_build::CodegenContext::new().capability(CAPABILITY)),
    )
    .expect("generate minimal Tauri context from the production Windows capability");
}
