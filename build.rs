fn main() {
    #[cfg(windows)]
    {
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .set("ProductName", "Nexode Orbit")
            .set("FileDescription", "Nexode Orbit Native Desktop")
            .compile()
            .expect("Unable to embed the Windows application icon");
    }
}
