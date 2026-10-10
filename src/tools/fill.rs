//! Заливка («ведро»): закрашивает область похожего цвета вокруг точки клика.
//!
//! Устроена как и кисть — реализует трейт Tool. Для отмены пользуется тем же Stroke:
//! «до» каждого изменённого пикселя кладётся в stroke.before, а history берёт его оттуда.

use crate::canvas::{allowed, Layer, Pixel, Rect};
use super::brush::blend;
use super::{Stroke, Tool, ToolInfo};

pub struct Fill {
    info: ToolInfo,
    color: Pixel,
    opacity: f32,
    /// Насколько цвет может отличаться от цвета под кликом и всё ещё залиться (0..255)
    tolerance: u8,
    /// true — только связная область вокруг клика; false — все похожие пиксели слоя
    contiguous: bool,
    /// Размер холста. Слой бесконечный — без границ заливка пустого места не закончилась бы
    bounds: (u32, u32),
    stroke: Option<Stroke>,
}

impl Fill {
    pub fn new(color: Pixel, opacity: f32, tolerance: u8, contiguous: bool, bounds: (u32, u32)) -> Self {
        Self {
            info: ToolInfo::new("Заливка", "Рисование", "Заливка области похожего цвета"),
            color,
            opacity: opacity.clamp(0.0, 1.0),
            tolerance,
            contiguous,
            bounds,
            stroke: None,
        }
    }
}

/// Насколько отличаются два пикселя: наибольшая разница по каналам (0..255)
fn diff(a: Pixel, b: Pixel) -> u8 {
    (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap()
}

impl Tool for Fill {
    fn info(&self) -> &ToolInfo {
        &self.info
    }

    fn begin_stroke(&mut self) {
        self.stroke = Some(Stroke::default());
    }

    fn end_stroke(&mut self) -> Option<Stroke> {
        self.stroke.take()
    }

    fn use_at(&mut self, layer: &mut Layer, pos: (f32, f32), clip: Option<Rect>) {
        let (w, h) = (self.bounds.0 as usize, self.bounds.1 as usize);
        let (cx, cy) = (pos.0.floor() as i32, pos.1.floor() as i32);
        if cx < 0 || cy < 0 || cx as usize >= w || cy as usize >= h || !allowed(clip, cx, cy) {
            return;
        }

        // Копия холста в плоский массив: дальше много раз читаем соседей,
        // и так быстрее, чем каждый раз искать блок в HashMap
        let src: Vec<Pixel> = (0..h as i32)
            .flat_map(|y| (0..w as i32).map(move |x| (x, y)))
            .map(|(x, y)| layer.get_pixel(x, y))
            .collect();
        let target = src[cy as usize * w + cx as usize];
        let tol = self.tolerance;

        // Сила заливки каждого пикселя 0..1. Внутри области — 1.
        // Пиксели на границе, которые чуть-чуть не прошли по допуску (мягкий край кисти),
        // заливаются частично — иначе вокруг мягких линий останется светлый ореол
        let mut weight = vec![0.0f32; w * h];

        if self.contiguous {
            // Обход от точки клика по соседям (как BFS по графу; в Python — while stack: ... pop()).
            // В стеке ещё «отличие родителя»: None — пришли изнутри области.
            let mut done = vec![false; w * h];
            let mut stack: Vec<(usize, usize, Option<u8>)> = vec![(cx as usize, cy as usize, None)];
            while let Some((x, y, parent)) = stack.pop() {
                let i = y * w + x;
                if done[i] {
                    continue;
                }
                let d = diff(src[i], target);
                if d <= tol {
                    weight[i] = 1.0; // внутри области
                } else {
                    // мягкий край: идём только «вверх по склону» — к линии, отличие растёт.
                    // Так ореол вокруг мягкой линии заливается, а в другие области не утекаем
                    if parent.is_some_and(|pd| d <= pd) {
                        continue;
                    }
                    let wgt = edge_weight(d, tol);
                    if wgt <= 0.0 {
                        continue; // дошли до самой линии
                    }
                    weight[i] = wgt;
                }
                done[i] = true;
                let from = if d <= tol { None } else { Some(d) };
                // соседи: слева, справа, сверху, снизу (checked_sub не даёт уйти ниже нуля)
                let neighbors = [
                    x.checked_sub(1).map(|nx| (nx, y)),
                    (x + 1 < w).then_some((x + 1, y)),
                    y.checked_sub(1).map(|ny| (x, ny)),
                    (y + 1 < h).then_some((x, y + 1)),
                ];
                for (nx, ny) in neighbors.into_iter().flatten() {
                    // граница выделения — такая же стена, как край холста
                    if !done[ny * w + nx] && allowed(clip, nx as i32, ny as i32) {
                        stack.push((nx, ny, from));
                    }
                }
            }
        } else {
            for (i, &p) in src.iter().enumerate() {
                if !allowed(clip, (i % w) as i32, (i / w) as i32) {
                    continue;
                }
                let d = diff(p, target);
                weight[i] = if d <= tol { 1.0 } else { edge_weight(d, tol) };
            }
        }

        for (i, &wgt) in weight.iter().enumerate() {
            if wgt <= 0.0 {
                continue;
            }
            let orig = src[i];
            let new = blend(orig, self.color, wgt * self.opacity);
            if new == orig {
                continue;
            }
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            if let Some(stroke) = &mut self.stroke {
                stroke.before.entry((x, y)).or_insert(orig);
            }
            layer.set_pixel(x, y, new);
        }
    }
}

/// Частичная заливка для пикселя, который не прошёл допуск на d - tol:
/// чуть-чуть не прошёл — почти полностью, сильно — не заливаем
fn edge_weight(d: u8, tol: u8) -> f32 {
    const SOFT: f32 = 96.0; // ширина «мягкой зоны» в единицах цвета
    (1.0 - (d - tol) as f32 / SOFT).clamp(0.0, 1.0)
}
