//! Icon and version information of mailcraft.exe (Windows only). The icon is drawn by the same
//! code as the window icon (src/art.rs), so there is no binary image in the repository.

#[cfg(windows)]
mod art {
    include!("src/art.rs");
}

fn main() {
    println!("cargo:rerun-if-changed=src/art.rs");
    #[cfg(windows)]
    {
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
        let ico_path = out.join("mailcraft.ico");
        let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
        for size in [16, 24, 32, 48, 64, 256] {
            let image = ico::IconImage::from_rgba_data(size, size, art::icon_rgba(size));
            dir.add_entry(ico::IconDirEntry::encode(&image).expect("icon entry"));
        }
        let file = std::fs::File::create(&ico_path).expect("icon file");
        dir.write(file).expect("icon written");

        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico_path.to_str().expect("utf-8 path"));
        res.set("ProductName", "MailCraft");
        res.set("FileDescription", "MailCraft");
        res.set(
            "LegalCopyright",
            "MailCraft contributors, MIT OR Apache-2.0",
        );
        res.compile().expect("Windows resources");
    }
}
