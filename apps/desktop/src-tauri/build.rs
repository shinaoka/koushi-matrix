fn main() {
    // One named compilation boundary for the desktop updater (#1035): the
    // shared lifecycle engine is compiled on every platform, while
    // `koushi_updater_backend` marks targets that ship a native install
    // backend. Windows currently uses this boundary for release notifications;
    // signed installer support can be added without changing shared lifecycle code.
    println!("cargo::rustc-check-cfg=cfg(koushi_updater_backend)");
    if matches!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("macos") | Ok("windows")
    ) {
        println!("cargo::rustc-cfg=koushi_updater_backend");
    }
    tauri_build::build()
}
