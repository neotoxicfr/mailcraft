//! MailCraft's icon, from its one source `assets/app-icon/mailcraft.svg` (rendered with resvg):
//! raw RGBA images for the window and the notification area (all systems), and on Windows the
//! .ico with the version information of mailcraft.exe (the installer takes the same .ico).

use resvg::{tiny_skia, usvg};

/// Sizes embedded for the program itself (window, notification area).
const RUNTIME: [u32; 2] = [32, 64];

fn render(svg: &str, size: u32) -> Vec<u8> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).expect("icon SVG");
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("icon size");
    let scale = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia keeps premultiplied alpha; icons want it straight.
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

fn main() {
    let source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/app-icon/mailcraft.svg"
    );
    println!("cargo:rerun-if-changed={source}");
    let svg = std::fs::read_to_string(source).expect("assets/app-icon/mailcraft.svg");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    for size in RUNTIME {
        std::fs::write(out.join(format!("icon-{size}.rgba")), render(&svg, size))
            .expect("icon pixels");
    }
    #[cfg(windows)]
    {
        let ico_path = out.join("mailcraft.ico");
        let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
        for size in [16, 24, 32, 48, 64, 256] {
            let image = ico::IconImage::from_rgba_data(size, size, render(&svg, size));
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
