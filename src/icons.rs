//! Small vector icons modeled on GitHub's Octicons, drawn with egui shapes on
//! a 16×16 grid so they stay sharp at any size.

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    PrOpen,
    PrDraft,
    PrMerged,
    PrClosed,
    Check,
    X,
    Dot,
    Skip,
    Comment,
    Commit,
    Checklist,
    FileDiff,
    Search,
    Sync,
    Eye,
    Link,
    /// Panel on the left, open: click to collapse.
    SidebarLeftClose,
    SidebarLeftOpen,
    SidebarRightClose,
    SidebarRightOpen,
    AutoMerge,
    /// "⋯" for menus.
    Kebab,
    Branch,
    Tag,
    Person,
    ChevronDown,
    ChevronRight,
    Copy,
    Folder,
    FileAdded,
    FileRemoved,
    FileMoved,
    FileModified,
    File,
    FolderOpen,
    Code,
}

pub fn show(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint(ui.painter(), rect, icon, color);
    resp
}

pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width() / 16.0;
    let at = |x: f32, y: f32| pos2(rect.left() + x * s, rect.top() + y * s);
    let w = (1.5 * s).max(1.0);
    let stroke = Stroke::new(w, color);
    let line = |pts: &[(f32, f32)]| {
        p.add(Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), stroke));
    };
    let ring = |x: f32, y: f32, r: f32| {
        p.circle_stroke(at(x, y), r * s, stroke);
    };
    match icon {
        Icon::PrOpen | Icon::PrDraft => {
            ring(4.25, 3.5, 1.75);
            ring(4.25, 12.5, 1.75);
            line(&[(4.25, 5.25), (4.25, 10.75)]);
            ring(11.75, 12.5, 1.75);
            if icon == Icon::PrDraft {
                for y in [3.5, 6.25, 9.0] {
                    p.circle_filled(at(11.75, y), 0.9 * s, color);
                }
            } else {
                line(&[(11.75, 10.75), (11.75, 6.0), (11.3, 4.6), (10.2, 3.8), (7.5, 3.5)]);
                line(&[(9.25, 1.75), (7.5, 3.5), (9.25, 5.25)]);
            }
        }
        Icon::PrClosed => {
            ring(4.25, 3.5, 1.75);
            ring(4.25, 12.5, 1.75);
            line(&[(4.25, 5.25), (4.25, 10.75)]);
            ring(11.75, 12.5, 1.75);
            line(&[(11.75, 10.75), (11.75, 7.5)]);
            line(&[(10.0, 2.0), (13.5, 5.5)]);
            line(&[(13.5, 2.0), (10.0, 5.5)]);
        }
        Icon::PrMerged => {
            ring(4.5, 3.25, 1.75);
            ring(4.5, 12.75, 1.75);
            line(&[(4.5, 5.0), (4.5, 11.0)]);
            ring(11.75, 9.5, 1.75);
            line(&[(4.5, 5.0), (4.9, 7.0), (6.2, 8.6), (7.8, 9.4), (10.0, 9.5)]);
        }
        Icon::Check => line(&[(2.75, 8.5), (6.25, 12.0), (13.25, 4.5)]),
        Icon::X => {
            line(&[(3.5, 3.5), (12.5, 12.5)]);
            line(&[(12.5, 3.5), (3.5, 12.5)]);
        }
        Icon::Dot => {
            p.circle_filled(at(8.0, 8.0), 4.0 * s, color);
        }
        Icon::Skip => {
            ring(8.0, 8.0, 5.75);
            line(&[(4.0, 12.0), (12.0, 4.0)]);
        }
        Icon::Comment => {
            let r = Rect::from_min_max(at(1.5, 2.25), at(14.5, 11.25));
            p.rect_stroke(r, 2.0 * s, stroke, egui::StrokeKind::Middle);
            line(&[(4.5, 11.25), (4.5, 14.25), (7.5, 11.25)]);
        }
        Icon::Commit => {
            ring(8.0, 8.0, 2.75);
            line(&[(0.75, 8.0), (5.25, 8.0)]);
            line(&[(10.75, 8.0), (15.25, 8.0)]);
        }
        Icon::Checklist => {
            let r = Rect::from_min_max(at(1.75, 1.75), at(14.25, 14.25));
            p.rect_stroke(r, 2.0 * s, stroke, egui::StrokeKind::Middle);
            line(&[(4.5, 8.25), (7.0, 10.75), (11.5, 5.5)]);
        }
        Icon::FileDiff => {
            line(&[(3.0, 1.75), (10.0, 1.75), (13.0, 4.75), (13.0, 14.25), (3.0, 14.25), (3.0, 1.75)]);
            line(&[(8.0, 4.5), (8.0, 9.0)]);
            line(&[(5.75, 6.75), (10.25, 6.75)]);
            line(&[(5.75, 11.5), (10.25, 11.5)]);
        }
        Icon::Search => {
            ring(6.75, 6.75, 4.75);
            line(&[(10.25, 10.25), (14.25, 14.25)]);
        }
        Icon::Sync => {
            let arc = |start: f32, end: f32| -> Vec<Pos2> {
                (0..=12)
                    .map(|i| {
                        let a = start + (end - start) * i as f32 / 12.0;
                        at(8.0 + 5.5 * a.cos(), 8.0 + 5.5 * a.sin())
                    })
                    .collect()
            };
            use std::f32::consts::PI;
            p.add(Shape::line(arc(PI * 1.1, PI * 1.9), stroke));
            p.add(Shape::line(arc(PI * 0.1, PI * 0.9), stroke));
            line(&[(12.6, 1.8), (12.7, 4.7), (9.9, 4.9)]);
            line(&[(3.4, 14.2), (3.3, 11.3), (6.1, 11.1)]);
        }
        Icon::Eye => {
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = i as f32 / 24.0 * std::f32::consts::TAU;
                    at(8.0 + 6.5 * a.cos(), 8.0 + 4.0 * a.sin())
                })
                .collect();
            p.add(Shape::closed_line(pts, stroke));
            p.circle_filled(at(8.0, 8.0), 1.75 * s, color);
        }
        Icon::SidebarLeftClose | Icon::SidebarLeftOpen | Icon::SidebarRightClose | Icon::SidebarRightOpen => {
            let r = Rect::from_min_max(at(1.25, 2.25), at(14.75, 13.75));
            p.rect_stroke(r, 2.0 * s, stroke, egui::StrokeKind::Middle);
            let left = matches!(icon, Icon::SidebarLeftClose | Icon::SidebarLeftOpen);
            let bar_x = if left { 5.75 } else { 10.25 };
            line(&[(bar_x, 2.25), (bar_x, 13.75)]);
            // Chevron in the big area, pointing the way the panel will move.
            let cx = if left { 10.0 } else { 6.0 };
            let points_left = matches!(icon, Icon::SidebarLeftClose | Icon::SidebarRightOpen);
            let d = if points_left { 1.25 } else { -1.25 };
            line(&[(cx + d, 5.5), (cx - d, 8.0), (cx + d, 10.5)]);
        }
        Icon::AutoMerge => {
            ring(4.5, 3.25, 1.75);
            ring(4.5, 12.75, 1.75);
            line(&[(4.5, 5.0), (4.5, 11.0)]);
            line(&[(4.5, 5.0), (4.9, 7.0), (6.2, 8.6), (7.8, 9.4), (9.0, 9.5)]);
            // A small clock: it will merge by itself later.
            ring(12.25, 9.5, 3.0);
            line(&[(12.25, 8.0), (12.25, 9.6), (13.3, 10.4)]);
        }
        Icon::Kebab => {
            for x in [3.0, 8.0, 13.0] {
                p.circle_filled(at(x, 8.0), 1.25 * s, color);
            }
        }
        Icon::Branch => {
            ring(4.5, 3.5, 1.75);
            ring(4.5, 12.5, 1.75);
            ring(11.5, 3.5, 1.75);
            line(&[(4.5, 5.25), (4.5, 10.75)]);
            line(&[(11.5, 5.25), (11.5, 6.5), (10.8, 8.0), (9.0, 8.8), (6.2, 9.4), (4.8, 10.6)]);
        }
        Icon::Tag => {
            line(&[(1.75, 2.75), (1.75, 7.75), (8.5, 14.5), (14.5, 8.5), (7.75, 1.75), (2.75, 1.75), (1.75, 2.75)]);
            p.circle_filled(at(5.25, 5.25), 1.1 * s, color);
        }
        Icon::Person => {
            ring(8.0, 5.0, 3.0);
            let pts: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 12.0);
                    at(8.0 + 5.75 * a.cos(), 14.5 + 5.0 * a.sin())
                })
                .collect();
            p.add(Shape::line(pts, stroke));
        }
        Icon::ChevronDown => line(&[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]),
        Icon::ChevronRight => line(&[(6.0, 4.0), (10.0, 8.0), (6.0, 12.0)]),
        Icon::Copy => {
            let back = Rect::from_min_max(at(1.75, 1.75), at(10.25, 10.25));
            p.rect_stroke(back, 1.5 * s, stroke, egui::StrokeKind::Middle);
            let front = Rect::from_min_max(at(5.75, 5.75), at(14.25, 14.25));
            p.rect_filled(front, 1.5 * s, Color32::TRANSPARENT);
            p.rect_stroke(front, 1.5 * s, stroke, egui::StrokeKind::Middle);
        }
        Icon::Folder => {
            p.add(Shape::convex_polygon(
                [(1.5, 3.0), (6.0, 3.0), (7.5, 4.75), (14.5, 4.75), (14.5, 13.25), (1.5, 13.25)].iter().map(|&(x, y)| at(x, y)).collect(),
                color,
                Stroke::NONE,
            ));
        }
        Icon::File => {
            line(&[(3.0, 1.75), (9.5, 1.75), (13.0, 5.25), (13.0, 14.25), (3.0, 14.25), (3.0, 1.75)]);
            line(&[(9.5, 1.75), (9.5, 5.25), (13.0, 5.25)]);
        }
        Icon::FolderOpen => {
            p.add(Shape::convex_polygon(
                [(1.5, 3.0), (6.0, 3.0), (7.5, 4.75), (13.0, 4.75), (13.0, 6.5), (1.5, 6.5)].iter().map(|&(x, y)| at(x, y)).collect(),
                color,
                Stroke::NONE,
            ));
            p.add(Shape::convex_polygon([(3.0, 7.5), (15.0, 7.5), (13.0, 13.25), (1.5, 13.25)].iter().map(|&(x, y)| at(x, y)).collect(), color, Stroke::NONE));
        }
        Icon::Code => {
            line(&[(5.0, 4.5), (1.75, 8.0), (5.0, 11.5)]);
            line(&[(11.0, 4.5), (14.25, 8.0), (11.0, 11.5)]);
        }
        Icon::FileModified => {
            let r = Rect::from_min_max(at(2.25, 2.25), at(13.75, 13.75));
            p.rect_stroke(r, 2.0 * s, stroke, egui::StrokeKind::Middle);
            p.circle_filled(at(8.0, 8.0), 2.0 * s, color);
        }
        Icon::FileAdded | Icon::FileRemoved | Icon::FileMoved => {
            let r = Rect::from_min_max(at(2.25, 2.25), at(13.75, 13.75));
            p.rect_stroke(r, 2.0 * s, stroke, egui::StrokeKind::Middle);
            match icon {
                Icon::FileAdded => {
                    line(&[(8.0, 5.0), (8.0, 11.0)]);
                    line(&[(5.0, 8.0), (11.0, 8.0)]);
                }
                Icon::FileRemoved => line(&[(5.0, 8.0), (11.0, 8.0)]),
                _ => {
                    line(&[(5.0, 8.0), (11.0, 8.0)]);
                    line(&[(8.5, 5.5), (11.0, 8.0), (8.5, 10.5)]);
                }
            }
        }
        Icon::Link => {
            line(&[(7.0, 9.0), (9.0, 7.0)]);
            line(&[(6.0, 7.0), (3.75, 9.25), (3.75, 11.5), (4.5, 12.25), (6.75, 12.25), (9.0, 10.0)]);
            line(&[(7.0, 6.0), (9.25, 3.75), (11.5, 3.75), (12.25, 4.5), (12.25, 6.75), (10.0, 9.0)]);
        }
    }
}
