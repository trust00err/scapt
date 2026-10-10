use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
/// Режимы наложения слоев
pub enum BlendMode {
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    Lighten,
    Screen,
    ColorDodge,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,

}

impl BlendMode{
    /// Все режимы по порядку. Номер в этом списке — то, что пишется в файл проекта,
    /// поэтому новые режимы добавлять только в КОНЕЦ, иначе старые файлы откроются неверно.
    pub const ALL: [BlendMode; 12] = [
        BlendMode::Normal,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
    ];

    /// Номер режима в ALL (для записи в файл)
    pub fn index(self) -> u8 {
        BlendMode::ALL.iter().position(|&m| m == self).unwrap() as u8
    }

    /// Режим по номеру из файла; None, если такого номера нет
    pub fn from_index(i: u8) -> Option<BlendMode> {
        BlendMode::ALL.get(i as usize).copied()
    }

    pub fn apply(self, cb: f32, cs: f32) -> f32 {
        match self {
            BlendMode::Normal => cs,
            BlendMode::Darken => cb.min(cs),
            BlendMode::Lighten => cb.max(cs),
            BlendMode::Multiply => cb * cs,
            BlendMode::Screen => 1.0-(1.0-cb)*(1.0-cs),
            BlendMode::ColorBurn => if cb == 1.0 {1.0} else if cs == 0.0 {0.0} else {1.0 - ((1.0 - cb) / cs).min(1.0)},
            BlendMode::ColorDodge => if cb == 0.0 { 0.0 } else if cs == 1.0 { 1.0 } else {(cb / (1.0 - cs)).min(1.0)},         
            BlendMode::HardLight => if cs <= 0.5 { BlendMode::Multiply.apply(cb, 2.0 * cs) } else { BlendMode::Screen.apply(cb, 2.0 * cs - 1.0) },
            BlendMode::Overlay => BlendMode::HardLight.apply(cs, cb),
            BlendMode::SoftLight => {if cs <= 0.5 {cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)} else {let d = if cb <= 0.25 { ((16.0 * cb - 12.0) * cb + 4.0) * cb } else { cb.sqrt() };cb + (2.0 * cs - 1.0) * (d - cb)}}
            BlendMode::Difference => (cb - cs).abs(),
            BlendMode::Exclusion => cb + cs - 2.0 * cb * cs,
        }
    }
}

/// Наложить пиксель src (верхний слой) на dst (нижний). Цвета premultiplied.
pub fn composite(dst: Pixel, src: Pixel, mode: BlendMode, opacity: f32) -> Pixel {
    let s = src.map(|p| p as f32 / 255.0 * opacity);
    let d = dst.map(|p| p as f32 / 255.0);

    let (s_a, d_a) = (s[3], d[3]);
    if s_a == 0.0 {
        return dst;
    }
    // Быстрые пути — без деления и без режима:
    // снизу пусто: при любом режиме остаётся верхний пиксель (с учётом opacity)
    if d_a == 0.0 {
        return s.map(|v| (v * 255.0).round() as u8);
    }
    // обычный режим и верхний полностью непрозрачен: он просто закрывает нижний
    if mode == BlendMode::Normal && s_a == 1.0 {
        return src;
    }
    else{
        let mut out = [0.0f32; 4];
        for i in 0..3 {
            // "чистые" цвета (без умножения на альфу) — нужны только для режима
            let cs = s[i] / s_a;
            let cb = if d_a > 0.0 { d[i] / d_a } else { 0.0 };
            out[i] = s[i] * (1.0 - d_a) + d[i] * (1.0 - s_a) + s_a * d_a * mode.apply(cb, cs);
        }
        out[3] = s_a + d_a * (1.0 - s_a);

        out.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
    }
}

/// Прямоугольник в пикселях холста: [x0, x1) × [y0, y1), правая и нижняя граница не входят.
/// Им задаётся выделение; инструменты не рисуют за его пределами
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    /// Прямоугольник по двум углам в любом порядке (тянуть выделение можно в любую сторону)
    pub fn from_corners(a: (i32, i32), b: (i32, i32)) -> Self {
        Self { x0: a.0.min(b.0), y0: a.1.min(b.1), x1: a.0.max(b.0), y1: a.1.max(b.1) }
    }

    pub fn width(&self) -> i32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> i32 {
        self.y1 - self.y0
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    /// Сдвинутая копия
    pub fn offset(&self, dx: i32, dy: i32) -> Self {
        Self { x0: self.x0 + dx, y0: self.y0 + dy, x1: self.x1 + dx, y1: self.y1 + dy }
    }

    /// Наименьший прямоугольник, содержащий оба
    pub fn union(&self, o: &Rect) -> Self {
        Self { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }

    /// Общая часть (может оказаться пустой)
    pub fn intersect(&self, o: &Rect) -> Self {
        Self { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) }
    }

    /// Все пиксели прямоугольника по строкам
    pub fn pixels(&self) -> impl Iterator<Item = (i32, i32)> {
        let (x0, x1) = (self.x0, self.x1);
        (self.y0..self.y1).flat_map(move |y| (x0..x1).map(move |x| (x, y)))
    }
}

/// Можно ли рисовать в (x, y): выделения нет — везде, есть — только внутри него
pub fn allowed(clip: Option<Rect>, x: i32, y: i32) -> bool {
    clip.is_none_or(|r| r.contains(x, y))
}

/// Цвет пикселя: [r, g, b, a]
pub type Pixel = [u8; 4];
pub const TRANSPARENT: Pixel = [0, 0, 0, 0];

/// Квадратный кусок холста с пикселями
#[derive(Clone)]
pub struct GridBlock {
    pub position: (i32, i32),
    pixels: Vec<Pixel>,
}

impl GridBlock {
    fn new(position: (i32, i32), size: usize) -> Self {
        Self {
            position,
            pixels: vec![TRANSPARENT; size * size],
        }
    }

    /// Получить пиксель по локальным координатам
    pub fn get(&self, x: usize, y: usize, size: usize) -> Pixel {
        self.pixels[y * size + x]
    }

    /// Блок из готовых пикселей (при загрузке проекта)
    pub fn from_pixels(position: (i32, i32), pixels: Vec<Pixel>) -> Self {
        Self { position, pixels }
    }

    /// Полностью прозрачный блок — такие не сохраняем
    pub fn is_empty(&self) -> bool {
        self.pixels.iter().all(|p| p[3] == 0)
    }

    /// Все пиксели блока подряд, индекс y * size + x
    pub fn pixels(&self) -> &[Pixel] {
        &self.pixels
    }

    /// Задать цвет пикселю по локальным координатам
    fn set(&mut self, x: usize, y: usize, size: usize, color: Pixel) {
        self.pixels[y * size + x] = color;
    }
}

/// Слой холста, разбитый на блоки.
/// Clone нужен истории: снимок слоёв до и после операции со слоями
#[derive(Clone)]
pub struct Layer {
    pub name: String,
    pub blend: BlendMode,   // режим наложения
    pub opacity: f32,       // прозрачность слоя 0..1
    pub visible: bool,      // видимость (глазик)
    block_size: usize,
    blocks: HashMap<(i32, i32), GridBlock>,
}

impl Layer {
    pub fn new(name: &str, block_size: usize) -> Self {
        Self {
            name: name.to_string(),
            blend: BlendMode::Normal,
            opacity: 1.0,
            visible: true,
            block_size,
            blocks: HashMap::new(),
        }
    }

    /// Переводит глобальные пиксельные координаты в (координаты блока, локальные x, y)
    pub fn locate(&self, x: i32, y: i32) -> ((i32, i32), usize, usize) {
        let s = self.block_size as i32;
        let block = (x.div_euclid(s), y.div_euclid(s));
        (block, x.rem_euclid(s) as usize, y.rem_euclid(s) as usize)
    }

    /// Задать цвет пикселю
    pub fn set_pixel(&mut self, x: i32, y: i32, color: Pixel) {
        let (block_pos, lx, ly) = self.locate(x, y);
        let size = self.block_size;
        let block = self
            .blocks
            .entry(block_pos)
            .or_insert_with(|| GridBlock::new(block_pos, size));
        block.set(lx, ly, size, color);
    }

    /// Получить пиксель по координатам
    pub fn get_pixel(&self, x: i32, y: i32) -> Pixel {
        let (block_pos, lx, ly) = self.locate(x, y);
        match self.blocks.get(&block_pos) {
            Some(block) => block.get(lx, ly, self.block_size),
            None => TRANSPARENT,
        }
    }

    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Все существующие блоки (в произвольном порядке — это HashMap)
    pub fn blocks(&self) -> impl Iterator<Item = &GridBlock> {
        self.blocks.values()
    }

    /// Положить готовый блок (заменяет старый на этом месте)
    pub fn insert_block(&mut self, block: GridBlock) {
        self.blocks.insert(block.position, block);
    }

    /// Блок по его координатам (в блоках, не в пикселях), если он существует
    pub fn block(&self, pos: (i32, i32)) -> Option<&GridBlock> {
        self.blocks.get(&pos)
    }

    /// Блок, в котором лежит пиксель (x, y), если он существует
    pub fn block_at(&self, x: i32, y: i32) -> Option<&GridBlock> {
        let (block_pos, _, _) = self.locate(x, y);
        self.blocks.get(&block_pos)
    }
}
