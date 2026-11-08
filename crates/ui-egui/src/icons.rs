//! Ribbon icons drawn in code (original work, MIT OR Apache-2.0): polylines in a unit square.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Envelope,
    Trash,
    Archive,
    Reply,
    ReplyAll,
    Forward,
    FolderPlus,
    Rename,
    Flag,
    Unsubscribe,
    Analyze,
    Sync,
    AddAccount,
    Exit,
    Paperclip,
    Gear,
    Rules,
    Away,
    Signature,
    Move,
    Bullets,
    Numbering,
    Link,
}

type Line = &'static [(f32, f32)];

const ENVELOPE: &[Line] = &[
    &[
        (0.1, 0.25),
        (0.9, 0.25),
        (0.9, 0.78),
        (0.1, 0.78),
        (0.1, 0.25),
    ],
    &[(0.1, 0.27), (0.5, 0.55), (0.9, 0.27)],
];
const TRASH: &[Line] = &[
    &[(0.15, 0.22), (0.85, 0.22)],
    &[(0.4, 0.22), (0.4, 0.12), (0.6, 0.12), (0.6, 0.22)],
    &[(0.22, 0.22), (0.28, 0.9), (0.72, 0.9), (0.78, 0.22)],
    &[(0.42, 0.35), (0.43, 0.78)],
    &[(0.58, 0.35), (0.57, 0.78)],
];
const ARCHIVE: &[Line] = &[
    &[(0.1, 0.2), (0.9, 0.2), (0.9, 0.38), (0.1, 0.38), (0.1, 0.2)],
    &[(0.16, 0.38), (0.16, 0.85), (0.84, 0.85), (0.84, 0.38)],
    &[(0.4, 0.52), (0.6, 0.52)],
];
const REPLY: &[Line] = &[
    &[(0.85, 0.82), (0.82, 0.6), (0.68, 0.48), (0.2, 0.48)],
    &[(0.4, 0.28), (0.2, 0.48), (0.4, 0.68)],
];
const REPLY_ALL: &[Line] = &[
    &[(0.9, 0.82), (0.87, 0.6), (0.73, 0.48), (0.3, 0.48)],
    &[(0.48, 0.28), (0.28, 0.48), (0.48, 0.68)],
    &[(0.3, 0.28), (0.1, 0.48), (0.3, 0.68)],
];
const FORWARD: &[Line] = &[
    &[(0.15, 0.82), (0.18, 0.6), (0.32, 0.48), (0.8, 0.48)],
    &[(0.6, 0.28), (0.8, 0.48), (0.6, 0.68)],
];
const FOLDER_PLUS: &[Line] = &[
    &[
        (0.1, 0.25),
        (0.4, 0.25),
        (0.48, 0.34),
        (0.9, 0.34),
        (0.9, 0.55),
    ],
    &[(0.6, 0.8), (0.1, 0.8), (0.1, 0.25)],
    &[(0.78, 0.6), (0.78, 0.95)],
    &[(0.6, 0.78), (0.96, 0.78)],
];
const RENAME: &[Line] = &[
    &[
        (0.2, 0.85),
        (0.25, 0.68),
        (0.72, 0.2),
        (0.85, 0.33),
        (0.37, 0.8),
        (0.2, 0.85),
    ],
    &[(0.63, 0.29), (0.76, 0.42)],
];
const FLAG: &[Line] = &[
    &[(0.25, 0.9), (0.25, 0.12)],
    &[
        (0.25, 0.15),
        (0.8, 0.15),
        (0.65, 0.33),
        (0.8, 0.52),
        (0.25, 0.52),
    ],
];
const UNSUBSCRIBE: &[Line] = &[
    &[
        (0.55, 0.72),
        (0.08, 0.72),
        (0.08, 0.22),
        (0.86, 0.22),
        (0.86, 0.45),
    ],
    &[(0.08, 0.24), (0.47, 0.5), (0.86, 0.24)],
    &[(0.66, 0.62), (0.92, 0.88)],
    &[(0.92, 0.62), (0.66, 0.88)],
];
const ANALYZE: &[Line] = &[&[(0.6, 0.56), (0.88, 0.85)]];
const SYNC: &[Line] = &[
    &[
        (0.2, 0.45),
        (0.28, 0.28),
        (0.45, 0.18),
        (0.65, 0.2),
        (0.8, 0.32),
    ],
    &[(0.8, 0.14), (0.8, 0.32), (0.62, 0.32)],
    &[
        (0.8, 0.55),
        (0.72, 0.72),
        (0.55, 0.82),
        (0.35, 0.8),
        (0.2, 0.68),
    ],
    &[(0.2, 0.86), (0.2, 0.68), (0.38, 0.68)],
];
const ADD_ACCOUNT: &[Line] = &[
    &[
        (0.1, 0.88),
        (0.13, 0.72),
        (0.24, 0.62),
        (0.4, 0.58),
        (0.56, 0.62),
    ],
    &[(0.78, 0.55), (0.78, 0.9)],
    &[(0.6, 0.72), (0.96, 0.72)],
];
const EXIT: &[Line] = &[
    &[(0.55, 0.15), (0.15, 0.15), (0.15, 0.85), (0.55, 0.85)],
    &[(0.4, 0.5), (0.9, 0.5)],
    &[(0.72, 0.32), (0.9, 0.5), (0.72, 0.68)],
];

/// A paper clip: outer loop, then the inner one.
const PAPERCLIP: &[Line] = &[&[
    (0.64, 0.34),
    (0.64, 0.74),
    (0.57, 0.87),
    (0.45, 0.87),
    (0.37, 0.75),
    (0.37, 0.24),
    (0.43, 0.12),
    (0.53, 0.12),
    (0.58, 0.23),
    (0.58, 0.7),
    (0.53, 0.77),
    (0.48, 0.73),
    (0.48, 0.34),
]];
/// Rules: a funnel (filtering incoming mail).
const RULES: &[Line] = &[&[
    (0.12, 0.18),
    (0.88, 0.18),
    (0.58, 0.52),
    (0.58, 0.84),
    (0.42, 0.92),
    (0.42, 0.52),
    (0.12, 0.18),
]];
/// Automatic replies: an envelope and a clock in front of it.
const AWAY: &[Line] = &[
    &[
        (0.44, 0.62),
        (0.06, 0.62),
        (0.06, 0.2),
        (0.7, 0.2),
        (0.7, 0.42),
    ],
    &[(0.06, 0.22), (0.38, 0.44), (0.7, 0.22)],
    &[(0.7, 0.56), (0.7, 0.7), (0.8, 0.76)],
];
/// Signature: a pen over a handwritten stroke.
const SIGNATURE: &[Line] = &[
    &[
        (0.42, 0.6),
        (0.47, 0.46),
        (0.8, 0.13),
        (0.91, 0.24),
        (0.58, 0.57),
        (0.42, 0.6),
    ],
    &[(0.72, 0.21), (0.83, 0.32)],
    &[
        (0.06, 0.84),
        (0.14, 0.74),
        (0.2, 0.84),
        (0.28, 0.76),
        (0.34, 0.86),
        (0.4, 0.8),
        (0.94, 0.8),
    ],
];
/// Bulleted list: the dots are circles, these are the lines.
const BULLETS: &[Line] = &[
    &[(0.32, 0.25), (0.92, 0.25)],
    &[(0.32, 0.5), (0.92, 0.5)],
    &[(0.32, 0.75), (0.92, 0.75)],
];
/// Numbered list: 1, 2, 3 and the lines.
const NUMBERING: &[Line] = &[
    &[(0.08, 0.18), (0.15, 0.13), (0.15, 0.33)],
    &[
        (0.07, 0.43),
        (0.11, 0.39),
        (0.17, 0.4),
        (0.18, 0.46),
        (0.07, 0.58),
        (0.19, 0.58),
    ],
    &[
        (0.07, 0.66),
        (0.18, 0.66),
        (0.12, 0.72),
        (0.18, 0.76),
        (0.16, 0.83),
        (0.07, 0.83),
    ],
    &[(0.32, 0.25), (0.92, 0.25)],
    &[(0.32, 0.5), (0.92, 0.5)],
    &[(0.32, 0.75), (0.92, 0.75)],
];
/// Link: two chain links.
const LINK: &[Line] = &[
    &[
        (0.44, 0.66),
        (0.3, 0.8),
        (0.2, 0.8),
        (0.12, 0.72),
        (0.12, 0.62),
        (0.3, 0.44),
        (0.4, 0.44),
        (0.47, 0.51),
    ],
    &[
        (0.56, 0.34),
        (0.7, 0.2),
        (0.8, 0.2),
        (0.88, 0.28),
        (0.88, 0.38),
        (0.7, 0.56),
        (0.6, 0.56),
        (0.53, 0.49),
    ],
    &[(0.38, 0.62), (0.62, 0.38)],
];
/// Move: a folder with an arrow going out of it.
const MOVE: &[Line] = &[
    &[
        (0.1, 0.25),
        (0.38, 0.25),
        (0.46, 0.34),
        (0.82, 0.34),
        (0.82, 0.44),
    ],
    &[(0.82, 0.7), (0.82, 0.8), (0.1, 0.8), (0.1, 0.25)],
    &[(0.36, 0.57), (0.96, 0.57)],
    &[(0.8, 0.44), (0.96, 0.57), (0.8, 0.7)],
];

impl Icon {
    #[cfg(test)]
    pub const ALL: [Self; 23] = [
        Self::Envelope,
        Self::Trash,
        Self::Archive,
        Self::Reply,
        Self::ReplyAll,
        Self::Forward,
        Self::FolderPlus,
        Self::Rename,
        Self::Flag,
        Self::Unsubscribe,
        Self::Analyze,
        Self::Sync,
        Self::AddAccount,
        Self::Exit,
        Self::Paperclip,
        Self::Gear,
        Self::Rules,
        Self::Away,
        Self::Signature,
        Self::Move,
        Self::Bullets,
        Self::Numbering,
        Self::Link,
    ];

    fn lines(self) -> &'static [Line] {
        match self {
            Self::Envelope => ENVELOPE,
            Self::Trash => TRASH,
            Self::Archive => ARCHIVE,
            Self::Reply => REPLY,
            Self::ReplyAll => REPLY_ALL,
            Self::Forward => FORWARD,
            Self::FolderPlus => FOLDER_PLUS,
            Self::Rename => RENAME,
            Self::Flag => FLAG,
            Self::Unsubscribe => UNSUBSCRIBE,
            Self::Analyze => ANALYZE,
            Self::Sync => SYNC,
            Self::AddAccount => ADD_ACCOUNT,
            Self::Exit => EXIT,
            Self::Paperclip => PAPERCLIP,
            Self::Gear => &[],
            Self::Rules => RULES,
            Self::Away => AWAY,
            Self::Signature => SIGNATURE,
            Self::Move => MOVE,
            Self::Bullets => BULLETS,
            Self::Numbering => NUMBERING,
            Self::Link => LINK,
        }
    }

    pub fn paint(self, painter: &Painter, rect: Rect, color: Color32) {
        let width = (rect.width() / 16.0).clamp(1.0, 2.0);
        let stroke = Stroke::new(width, color);
        let at = |(x, y): (f32, f32)| -> Pos2 {
            pos2(
                rect.left() + x * rect.width(),
                rect.top() + y * rect.height(),
            )
        };
        for line in self.lines() {
            painter.add(Shape::line(line.iter().copied().map(at).collect(), stroke));
        }
        let size = rect.width().min(rect.height());
        for &(x, y, r) in self.circles() {
            painter.circle_stroke(at((x, y)), r * size, stroke);
        }
        if self == Self::Gear {
            // Eight teeth around a ring, and the hole.
            let c = at((0.5, 0.5));
            let (outer, inner) = (0.44 * size, 0.33 * size);
            let mut points = Vec::with_capacity(33);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                for (da, r) in [(-0.3, inner), (-0.17, outer), (0.17, outer), (0.3, inner)] {
                    let t = a + da;
                    points.push(c + egui::vec2(t.cos(), t.sin()) * r);
                }
            }
            points.push(points[0]);
            painter.add(Shape::line(points, stroke));
            painter.circle_stroke(c, 0.13 * size, stroke);
        }
    }

    /// Circles: (centre x, centre y, radius) in the unit square.
    fn circles(self) -> &'static [(f32, f32, f32)] {
        match self {
            Self::Analyze => &[(0.4, 0.37, 0.25)],
            Self::AddAccount => &[(0.4, 0.31, 0.17)],
            Self::Away => &[(0.7, 0.68, 0.22)],
            Self::Bullets => &[(0.15, 0.25, 0.05), (0.15, 0.5, 0.05), (0.15, 0.75, 0.05)],
            _ => &[],
        }
    }
}

#[cfg(test)]
mod gallery {
    use super::Icon;

    /// Every icon at ribbon size and large, for a visual review (needs a GPU).
    #[test]
    #[ignore = "writes target/shots/icons.png"]
    fn icons_sheet() {
        let all = Icon::ALL;
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(120.0 * all.len() as f32 / 2.0, 330.0))
            .wgpu()
            .build_ui(move |ui| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, egui::Color32::WHITE);
                let ink = egui::Color32::from_rgb(0x0F, 0x6C, 0xBD);
                for (i, icon) in all.iter().enumerate() {
                    let col = (i % (all.len().div_ceil(2))) as f32;
                    let row = (i / all.len().div_ceil(2)) as f32;
                    let x = 10.0 + col * 120.0;
                    let y = 10.0 + row * 160.0;
                    icon.paint(
                        ui.painter(),
                        egui::Rect::from_min_size(egui::pos2(x, y), egui::Vec2::splat(30.0)),
                        ink,
                    );
                    icon.paint(
                        ui.painter(),
                        egui::Rect::from_min_size(egui::pos2(x + 40.0, y), egui::Vec2::splat(64.0)),
                        ink,
                    );
                    ui.painter().text(
                        egui::pos2(x, y + 110.0),
                        egui::Align2::LEFT_TOP,
                        format!("{icon:?}"),
                        egui::FontId::proportional(13.0),
                        egui::Color32::BLACK,
                    );
                }
            });
        h.run();
        let image = h.render().expect("render");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
        std::fs::create_dir_all(&dir).unwrap();
        image.save(dir.join("icons.png")).unwrap();
    }
}
