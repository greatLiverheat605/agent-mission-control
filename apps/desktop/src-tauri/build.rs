fn main() {
    let icon_path = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"))
        .join("mission-control.ico");
    std::fs::write(&icon_path, PLACEHOLDER_ICON).expect("write build icon");
    let mut overlay = std::env::var("TAURI_CONFIG")
        .ok()
        .map(|value| serde_json::from_str(&value).expect("parse TAURI_CONFIG"))
        .unwrap_or_else(|| serde_json::json!({}));
    overlay["bundle"]["icon"] = serde_json::json!([icon_path]);
    let overlay = serde_json::to_string(&overlay).expect("serialize TAURI_CONFIG");
    unsafe { std::env::set_var("TAURI_CONFIG", &overlay) };
    println!("cargo:rustc-env=TAURI_CONFIG={overlay}");
    let windows = tauri_build::WindowsAttributes::new().window_icon_path(icon_path);
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("build Tauri context");
}

// A one-pixel ICO keeps Phase 1 builds valid until packaging supplies branded assets.
const PLACEHOLDER_ICON: &[u8] = &[
    0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x20, 0x00, 0x30, 0x00,
    0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x01, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x72, 0x7d,
    0x14, 0xff, 0x00, 0x00, 0x00, 0x00,
];
