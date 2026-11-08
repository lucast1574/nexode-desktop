fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
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
