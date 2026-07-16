fn main() {
    const CAPABILITY: &str = "../../apps/desktop/src-tauri/capabilities/windows-overlay.json";

    println!("cargo:rerun-if-changed={CAPABILITY}");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(
                tauri_build::WindowsAttributes::new()
                    .window_icon_path("../../apps/desktop/src-tauri/icons/icon.ico"),
            )
            .capabilities_path_pattern(CAPABILITY)
            .codegen(tauri_build::CodegenContext::new().capability(CAPABILITY)),
    )
    .expect("generate minimal Tauri context from the production Windows capability");
}
