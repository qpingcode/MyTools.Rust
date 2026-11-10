fn main() -> std::io::Result<()> {
    mytools_plugin_api::crash::install();
    mytools_encode::run(
        true,
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
}
