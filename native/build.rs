fn main() {
    // Compileaza interfata Slint.
    slint_build::compile("ui/main.slint").expect("Slint UI compilation failed");

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
