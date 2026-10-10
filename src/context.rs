use crate::canvas::{composite, Layer, Pixel, Rect, TRANSPARENT};
use crate::tools::{Cursor, Stroke, Tool};

/// Состояние открытого проекта: размеры, слои, курсор.
pub struct Context {
    pub project_name: String,
    pub width: u32,
    pub height: u32,
    pub block_size: usize,
    pub layers: Vec<Layer>,
    pub active_layer: usize,   // индекс слоя, в котором рисуем
    pub cursor: Cursor,
    pub selection: Option<Rect>, // выделение: инструменты рисуют только внутри него
}

impl Context {
    pub fn new(project_name: &str, width: u32, height: u32, tool: Box<dyn Tool>) -> Self {
        let mut ctx = Self {
            project_name: project_name.to_string(),
            width,
            height,
            block_size: 64,
            layers: Vec::new(),
            active_layer: 0,
            cursor: Cursor::new(tool),
            selection: None,
        };
        ctx.add_layer("Background");   // у нового проекта всегда есть хотя бы один слой
        ctx
    }

    /// Добавить слой и сделать его активным
    pub fn add_layer(&mut self, name: &str) {
        self.layers.push(Layer::new(name, self.block_size));
        self.active_layer = self.layers.len() - 1;
    }

    /// Выбрать активный слой
    pub fn select_layer(&mut self, index: usize) {
        if index < self.layers.len() {
            self.active_layer = index;
        }
    }

    /// Обновить позицию курсора (вызывается при движении мыши)
    pub fn move_cursor(&mut self, x: f32, y: f32) {
        self.cursor.position = (x, y);
    }

    /// Клик: применить инструмент к активному слою
    pub fn click(&mut self) {
        let layer = &mut self.layers[self.active_layer];
        self.cursor.click(layer, self.selection);
    }

    /// Кнопку мыши нажали: начинается мазок
    pub fn begin_stroke(&mut self) {
        self.cursor.begin_stroke();
    }

    /// Кнопку мыши отпустили: мазок закончен. Возвращает его — для отмены
    pub fn end_stroke(&mut self) -> Option<Stroke> {
        self.cursor.end_stroke()
    }

    /// Собрать целый блок pos из всех видимых слоёв снизу вверх.
    /// Быстрее, чем composite_pixel для каждого пикселя: каждый слой ищет блок
    /// в HashMap один раз, а слой без этого блока (пустое место) пропускается целиком.
    pub fn composite_block(&self, pos: (i32, i32), out: &mut Vec<Pixel>) {
        out.clear();
        out.resize(self.block_size * self.block_size, TRANSPARENT);
        for layer in self.layers.iter().filter(|layer| layer.visible) {
            if let Some(block) = layer.block(pos) {
                for (dst, &src) in out.iter_mut().zip(block.pixels()) {
                    *dst = composite(*dst, src, layer.blend, layer.opacity);
                }
            }
        }
    }

    /// Итоговый цвет пикселя: все видимые слои снизу вверх
    pub fn composite_pixel(&self, x: i32, y: i32) -> Pixel {
        self.layers
            .iter()
            .filter(|layer| layer.visible)
            .fold(TRANSPARENT, |below, layer| {
                composite(below, layer.get_pixel(x, y), layer.blend, layer.opacity)
            })
    }

    /// Сменить инструмент
    pub fn set_tool(&mut self, tool: Box<dyn Tool>) {
        self.cursor.set_tool(tool);
    }
}