fn main() -> std::io::Result<()> {
    mytools_encode::run(
        false,
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
}
