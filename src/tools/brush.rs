use crate::canvas::{allowed, Layer, Pixel, Rect};
use super::curve::BoundaryFunc;
use super::shape::ShapeMask;
use super::{Stroke, Tool, ToolInfo};

/// Что делает отпечаток: кладёт краску или стирает
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaintMode {
    Paint,
    Erase,
}

pub struct Brush {
    info: ToolInfo,
    color: Pixel,
    radius: i32,
    opacity: f32,
    hardness: BoundaryFunc,
    use_hardness: bool,
    /// Форма кисти, ужатая под радиус (см. BrushShape::to_mask).
    /// None — обычная круглая кисть.
    figure: Option<ShapeMask>,
    /// Текущий мазок; None — кнопка мыши не нажата
    stroke: Option<Stroke>,
    /// Кисть или ластик: всё остальное (форма, жёсткость, мазок) у них общее
    mode: PaintMode,
}

impl Brush {
    pub fn new(
        name: &str,
        group: &str,
        description: &str,
        color: Pixel,
        radius: i32,
        opacity: f32,
        hardness: BoundaryFunc,
        figure: Option<ShapeMask>,
    ) -> Result<Self, String> {
        let radius = radius.max(1);
        let side = (2 * radius + 1) as usize;

        if let Some(f) = &figure {
            if f.side != side {
                return Err(format!(
                    "маска должна быть {side}×{side} под радиус {radius}, а она {}×{}",
                    f.side, f.side
                ));
            }
        }

        Ok(Self {
            info: ToolInfo::new(name, group, description),
            color,
            radius,
            opacity: opacity.clamp(0.0, 1.0),
            hardness,
            use_hardness: true,
            figure,
            stroke: None,
            mode: PaintMode::Paint,
        })
    }

    pub fn set_color(&mut self, color: Pixel) {
        self.color = color;
    }

    pub fn set_use_hardness(&mut self, on: bool) {
        self.use_hardness = on;
    }

    pub fn set_mode(&mut self, mode: PaintMode) {
        self.mode = mode;
    }

    /// Результат одного пикселя: исходный цвет orig + отпечаток силой alpha
    fn apply(&self, orig: Pixel, alpha: f32) -> Pixel {
        match self.mode {
            PaintMode::Paint => blend(orig, self.color, alpha),
            PaintMode::Erase => erase(orig, alpha),
        }
    }

    /// Непрозрачность кисти в точке (dx, dy) относительно центра: 0..1
    /// (dx, dy) — от центра отпечатка до центра пикселя, дробные
    fn alpha_at(&self, dx: f32, dy: f32) -> f32 {
        let r = self.radius;

        // shape — сколько краски даёт форма, t — где мы между «центром» (0) и краем (1)
        let (shape, t) = match &self.figure {
            // форма из картинки: берём ближайшую клетку маски.
            // t — по расстоянию до её контура (вариант В)
            Some(mask) => {
                let (ix, iy) = (dx.round() as i32 + r, dy.round() as i32 + r);
                if ix < 0 || iy < 0 || ix as usize >= mask.side || iy as usize >= mask.side {
                    return 0.0;
                }
                let i = iy as usize * mask.side + ix as usize;
                (mask.shape[i], mask.t[i])
            }
            // круг: t по расстоянию до центра. Край сглажен на 1 пиксель:
            // пиксель на границе закрашивается частично, а не «да/нет»
            None => {
                let dist = (dx * dx + dy * dy).sqrt();
                let shape = (r as f32 + 0.5 - dist).clamp(0.0, 1.0);
                (shape, (dist / r as f32).min(1.0))
            }
        };

        let hard = if self.use_hardness { self.hardness.sample(t) } else { 1.0 };

        shape * hard * self.opacity
    }
}

impl Tool for Brush {
    fn info(&self) -> &ToolInfo {
        &self.info
    }

    fn begin_stroke(&mut self) {
        self.stroke = Some(Stroke::default());
    }

    fn end_stroke(&mut self) -> Option<Stroke> {
        // take: забрать значение из Option, оставив на его месте None
        self.stroke.take()
    }

    fn use_at(&mut self, layer: &mut Layer, pos: (f32, f32), clip: Option<Rect>) {
        // все пиксели, которые может задеть отпечаток радиуса r (+ полпикселя сглаживания)
        let reach = self.radius as f32 + 0.5;
        let (x0, x1) = ((pos.0 - reach).floor() as i32, (pos.0 + reach).floor() as i32);
        let (y0, y1) = ((pos.1 - reach).floor() as i32, (pos.1 + reach).floor() as i32);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if !allowed(clip, x, y) {
                    continue; // за пределами выделения не рисуем
                }
                // от центра отпечатка до центра пикселя (центр пикселя x — это x + 0.5).
                // Считаем ДО того, как возьмём &mut self.stroke (см. ловушку заимствования)
                let alpha = self.alpha_at(x as f32 + 0.5 - pos.0, y as f32 + 0.5 - pos.1);
                if alpha <= 0.0 {
                    continue;
                }

                match &mut self.stroke {
                    // идёт мазок: максимум покрытия, смешиваем с цветом ДО мазка
                    Some(stroke) => {
                        let covered = stroke.coverage.get(&(x, y)).copied().unwrap_or(0.0);
                        if alpha <= covered {
                            continue; // уже закрашено сильнее
                        }
                        stroke.coverage.insert((x, y), alpha);
                        let orig = *stroke.before.entry((x, y)).or_insert_with(|| layer.get_pixel(x, y));
                        layer.set_pixel(x, y, self.apply(orig, alpha));
                    }
                    // одиночный клик без мазка: как раньше, накапливаем
                    None => {
                        let old = layer.get_pixel(x, y);
                        layer.set_pixel(x, y, self.apply(old, alpha));
                    }
                }
            }
        }
    }
}

/// Стирание: все четыре канала умножаются на (1 - a).
/// Цвета premultiplied, поэтому это ровно «сделать пиксель прозрачнее», без сдвига цвета.
fn erase(old: Pixel, a: f32) -> Pixel {
    old.map(|c| (c as f32 * (1.0 - a)).round() as u8)
}

/// Смешивание: old * (1 - a) + new * a, по каждому каналу
pub(crate) fn blend(old: Pixel, new: Pixel, a: f32) -> Pixel {
    let a = a * (new[3] as f32 / 255.0);
    let mut out = [0u8; 4];
    for i in 0..4 {
        let v = old[i] as f32 * (1.0 - a) + new[i] as f32 * a;
        out[i] = v.round().clamp(0.0, 255.0) as u8;
    }
    out
}