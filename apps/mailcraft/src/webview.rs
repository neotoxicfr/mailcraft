//! "Show original": the message's own HTML rendered by Edge WebView2, as a child of the main
//! window placed over the reading pane. JavaScript is off, the page cannot navigate (links open
//! in the default browser), new windows are refused, devtools are off.

use eframe::egui;
use wry::dpi::{PhysicalPosition, PhysicalSize};
use wry::{NewWindowResponse, Rect, WebContext, WebView, WebViewBuilder};

pub struct OriginalView {
    view: Option<WebView>,
    context: WebContext,
    html: u64,
    bounds: Option<Rect>,
    visible: bool,
    failed: bool,
    /// wry's container window, found once: the region cut out of it.
    container: Option<winsafe::HWND>,
    /// The menus currently cut out of the view (pixels, in the view's own coordinates).
    holes: Vec<[i32; 4]>,
}

/// wry's container window around the WebView (class `WRY_WEBVIEW`), among the children of this
/// thread's windows.
fn find_container() -> Option<winsafe::HWND> {
    let mut found = None;
    let _ = winsafe::EnumThreadWindows(winsafe::GetCurrentThreadId(), |top: winsafe::HWND| {
        top.EnumChildWindows(|child: winsafe::HWND| {
            if child.GetClassName().is_ok_and(|c| c == "WRY_WEBVIEW") {
                found = Some(child);
                return false;
            }
            true
        });
        found.is_none()
    });
    found
}

/// The view's shape: its whole rectangle minus the holes, or no shape (all of it) without holes.
fn shape(
    container: &winsafe::HWND,
    size: (i32, i32),
    holes: &[[i32; 4]],
) -> winsafe::SysResult<()> {
    use winsafe::prelude::Handle as _;
    use winsafe::{HRGN, RECT, co};
    if holes.is_empty() {
        return container.SetWindowRgn(&HRGN::NULL, true);
    }
    let rect = |[left, top, right, bottom]: [i32; 4]| RECT {
        left,
        top,
        right,
        bottom,
    };
    let mut region = HRGN::CreateRectRgn(rect([0, 0, size.0, size.1]))?;
    for h in holes {
        let hole = HRGN::CreateRectRgn(rect(*h))?;
        region.CombineRgn(&region, &hole, co::RGN::DIFF)?;
    }
    container.SetWindowRgn(&region, true)?;
    // The system owns the region once it is set.
    let _ = region.leak();
    Ok(())
}

/// Links clicked in the page open in the user's browser, never inside MailCraft.
fn open_outside(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        let _ = open::that_detached(url);
        return true;
    }
    false
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

impl OriginalView {
    pub fn new(data_dir: Option<std::path::PathBuf>) -> Self {
        Self {
            view: None,
            context: WebContext::new(data_dir),
            html: 0,
            bounds: None,
            visible: false,
            failed: false,
            container: None,
            holes: Vec::new(),
        }
    }

    /// Call every frame: `want` is the reading pane's rectangle (points) and the HTML to show,
    /// `None` to hide the view; `menus` are the open menus' rectangles (points).
    pub fn update(
        &mut self,
        frame: &eframe::Frame,
        ppp: f32,
        want: Option<(egui::Rect, &str)>,
        menus: &[egui::Rect],
    ) {
        let Some((rect, html)) = want else {
            if self.visible
                && let Some(v) = &self.view
            {
                let _ = v.set_visible(false);
            }
            self.visible = false;
            return;
        };
        if self.failed {
            return;
        }
        let bounds = Rect {
            position: PhysicalPosition::new(
                (rect.min.x * ppp).round() as i32,
                (rect.min.y * ppp).round() as i32,
            )
            .into(),
            size: PhysicalSize::new(
                (rect.width() * ppp).round().max(1.0) as u32,
                (rect.height() * ppp).round().max(1.0) as u32,
            )
            .into(),
        };
        let h = hash(html);
        match &self.view {
            None => {
                let built = WebViewBuilder::new_with_web_context(&mut self.context)
                    .with_html(html)
                    .with_bounds(bounds)
                    .with_javascript_disabled()
                    .with_devtools(false)
                    .with_autoplay(false)
                    // The first load is the HTML itself; any later navigation is a click on a link.
                    .with_navigation_handler(|url| !open_outside(&url))
                    .with_new_window_req_handler(|url, _| {
                        open_outside(&url);
                        NewWindowResponse::Deny
                    })
                    .build_as_child(frame);
                match built {
                    Ok(v) => {
                        self.view = Some(v);
                        self.html = h;
                        self.bounds = Some(bounds);
                        self.visible = true;
                    }
                    Err(e) => {
                        log::warn!("original view unavailable: {e}");
                        self.failed = true;
                    }
                }
            }
            Some(v) => {
                if self.html != h {
                    let _ = v.load_html(html);
                    self.html = h;
                }
                if self.bounds != Some(bounds) {
                    let _ = v.set_bounds(bounds);
                    self.bounds = Some(bounds);
                    // Resized: the shape is made again below.
                    self.holes.push([0; 4]);
                }
                if !self.visible {
                    let _ = v.set_visible(true);
                    self.visible = true;
                }
            }
        }
        self.cut_menus(rect, ppp, menus);
    }

    /// The view sits above everything MailCraft draws: the menus that overlap it (right-click,
    /// drop-downs, tooltips) are cut out of it, so they show over the mail like in Outlook.
    fn cut_menus(&mut self, rect: egui::Rect, ppp: f32, menus: &[egui::Rect]) {
        if self.view.is_none() {
            return;
        }
        let px = |v: f32| (v * ppp).round() as i32;
        let holes: Vec<[i32; 4]> = menus
            .iter()
            .filter(|m| m.intersects(rect))
            .map(|m| {
                [
                    px(m.min.x - rect.min.x),
                    px(m.min.y - rect.min.y),
                    px(m.max.x - rect.min.x),
                    px(m.max.y - rect.min.y),
                ]
            })
            .collect();
        if holes == self.holes {
            return;
        }
        if self.container.is_none() {
            self.container = find_container();
        }
        let Some(container) = &self.container else {
            return;
        };
        match shape(container, (px(rect.width()), px(rect.height())), &holes) {
            Ok(()) => self.holes = holes,
            Err(e) => log::warn!("original view: menus not cut out ({e})"),
        }
    }

    pub fn failed(&self) -> bool {
        self.failed
    }
}
