fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");
        res.set("ProductName", "OmniFind");
        res.set("FileDescription", "OmniFind - High Performance Desktop Search Engine");
        res.set("LegalCopyright", "Copyright (C) 2026 Waleed Tahir");
        if let Err(e) = res.compile() {
            eprintln!("Warning: Failed to compile windows resource: {}", e);
        }
    }
}
