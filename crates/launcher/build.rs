fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles);
    slint_build::compile_with_config("ui/main.slint", config).expect("无法编译启动器 Slint UI");
    embed_windows_icon();
}

/// Embeds `ui/assets/app.ico` as icon resource 1, which Explorer, the taskbar and pinned
/// shortcuts show for mytools.exe; `desktop::show` also loads it as the taskbar icon.
fn embed_windows_icon() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../ui/assets/app.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    winresource::WindowsResource::new()
        .set_icon(icon.to_str().expect("图标路径不是有效的 UTF-8"))
        .set("ProductName", "MyTools")
        .set("FileDescription", "MyTools")
        .compile()
        .expect("无法嵌入 Windows 图标资源");
}
