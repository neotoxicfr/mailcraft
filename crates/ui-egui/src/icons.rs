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
    Folder,
    FolderPlus,
    Rename,
    Flag,
    Unsubscribe,
    Analyze,
    Sync,
    AddAccount,
    Exit,
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
const FOLDER: &[Line] = &[&[
    (0.1, 0.25),
    (0.4, 0.25),
    (0.48, 0.34),
    (0.9, 0.34),
    (0.9, 0.8),
    (0.1, 0.8),
    (0.1, 0.25),
]];
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
const ANALYZE: &[Line] = &[
    &[
        (0.42, 0.12),
        (0.6, 0.18),
        (0.68, 0.36),
        (0.6, 0.55),
        (0.42, 0.62),
        (0.24, 0.55),
        (0.16, 0.36),
        (0.24, 0.18),
        (0.42, 0.12),
    ],
    &[(0.6, 0.55), (0.88, 0.85)],
];
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
        (0.4, 0.15),
        (0.53, 0.2),
        (0.57, 0.33),
        (0.53, 0.46),
        (0.4, 0.5),
        (0.27, 0.46),
        (0.23, 0.33),
        (0.27, 0.2),
        (0.4, 0.15),
    ],
    &[(0.1, 0.88), (0.15, 0.68), (0.4, 0.58), (0.6, 0.62)],
    &[(0.78, 0.55), (0.78, 0.9)],
    &[(0.6, 0.72), (0.96, 0.72)],
];
const EXIT: &[Line] = &[
    &[(0.55, 0.15), (0.15, 0.15), (0.15, 0.85), (0.55, 0.85)],
    &[(0.4, 0.5), (0.9, 0.5)],
    &[(0.72, 0.32), (0.9, 0.5), (0.72, 0.68)],
];

impl Icon {
    fn lines(self) -> &'static [Line] {
        match self {
            Self::Envelope => ENVELOPE,
            Self::Trash => TRASH,
            Self::Archive => ARCHIVE,
            Self::Reply => REPLY,
            Self::ReplyAll => REPLY_ALL,
            Self::Forward => FORWARD,
            Self::Folder => FOLDER,
            Self::FolderPlus => FOLDER_PLUS,
            Self::Rename => RENAME,
            Self::Flag => FLAG,
            Self::Unsubscribe => UNSUBSCRIBE,
            Self::Analyze => ANALYZE,
            Self::Sync => SYNC,
            Self::AddAccount => ADD_ACCOUNT,
            Self::Exit => EXIT,
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
    }
}
