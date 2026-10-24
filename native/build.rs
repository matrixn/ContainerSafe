fn main() {
    // Compileaza interfata Slint.
    slint_build::compile_with_config(
        "ui/main.slint",
        slint_build::CompilerConfiguration::new()
            .with_bundled_translations("translations")
            .with_default_translation_context(slint_build::DefaultTranslationContext::None),
    )
    .expect("Slint UI compilation failed");

    // Adauga iconita in executabilul Windows.
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();

        res.set_icon("assets/icons/containersafe.ico");
        res.set("ProductName", "ContainerSafe");
        res.set("FileDescription", "ContainerSafe Docker Backup Manager");

        res.compile().expect("Windows resource compilation failed");
    }

    println!("cargo:rerun-if-changed=assets/icons/containersafe.ico");
}
