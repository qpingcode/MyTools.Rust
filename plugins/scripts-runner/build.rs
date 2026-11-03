fn main() {
    slint_build::compile_with_config(
        "ui/manager.slint",
        slint_build::CompilerConfiguration::new()
            .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles),
    )
    .expect("script manager UI");
}
