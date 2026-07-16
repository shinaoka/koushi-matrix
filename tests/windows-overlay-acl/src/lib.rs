#[cfg(all(test, target_os = "windows"))]
mod tests {
    use tauri::webview::InvokeRequest;

    #[test]
    fn windows_overlay_ipc_is_authorized() {
        let app = tauri::test::mock_builder()
            .build(tauri::tauri_build_context!())
            .expect("build isolated app from the production Windows capability");
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("build main mock webview");

        for body in [
            serde_json::json!({
                "label": "main",
                "value": { "rgba": [255, 0, 0, 255], "width": 1, "height": 1 }
            }),
            serde_json::json!({ "label": "main", "value": null }),
        ] {
            let response = tauri::test::get_ipc_response(
                &webview,
                InvokeRequest {
                    cmd: "plugin:window|set_overlay_icon".into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: "http://tauri.localhost".parse().expect("valid local URL"),
                    body: tauri::ipc::InvokeBody::Json(body),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.to_owned(),
                },
            );
            assert!(
                response.is_ok(),
                "overlay IPC must cross the production ACL: {response:?}"
            );
        }

        println!("windows_overlay_ipc=ok");
    }
}
