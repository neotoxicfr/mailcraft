// MailCraft's icon (assets/app-icon/mailcraft.svg, original work, MIT OR Apache-2.0), rendered
// by build.rs for the window and the notification area.

/// RGBA pixels, `size`×`size`; 32 and 64 are rendered at build time, other sizes use 64.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    match size {
        32 => include_bytes!(concat!(env!("OUT_DIR"), "/icon-32.rgba")).to_vec(),
        _ => include_bytes!(concat!(env!("OUT_DIR"), "/icon-64.rgba")).to_vec(),
    }
}
