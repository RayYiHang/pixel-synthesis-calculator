fn main() {
    // Windows 可执行文件嵌入图标与版本信息。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/icon.ico");
        res.compile().expect("failed to compile Windows resources");
    }
}
