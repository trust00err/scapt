pub mod brush;
pub mod curve;
pub mod fill;
pub mod shape;

use std::collections::HashMap;

use crate::canvas::{GridBlock, Layer, Pixel, Rect};

/// Общие данные инструмента
pub struct ToolInfo {
    pub name: String,
    pub group: String,
    pub description: String,
}

impl ToolInfo {
    pub fn new(name: &str, group: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            group: group.to_string(),
            description: description.to_string(),
        }
    }
}

/// Один мазок: от нажатия кнопки мыши до отпускания.
/// Каждый пиксель за мазок получает максимум покрытия, а не сумму отпечатков.
#[derive(Default)]
pub struct Stroke {
    /// Самое сильное покрытие, которое пиксель уже получил в этом мазке (0..1)
    pub coverage: HashMap<(i32, i32), f32>,
    /// Каким был пиксель до начала мазка (пригодится и для отмены)
    pub before: HashMap<(i32, i32), Pixel>,
}

/// Инструмент, который управляется курсором
pub trait Tool {
    fn info(&self) -> &ToolInfo;
    /// Применить инструмент в точке pos. Координаты дробные: центр отпечатка не
    /// прилипает к сетке пикселей, иначе край мазка идёт ступеньками («бусинами»)
    /// clip — выделение: за его пределами инструмент не рисует (None — рисовать везде)
    fn use_at(&mut self, layer: &mut Layer, pos: (f32, f32), clip: Option<Rect>);
    /// Начало мазка (кнопку нажали). По умолчанию ничего не делает
    fn begin_stroke(&mut self) {}
    /// Конец мазка (кнопку отпустили). Отдаёт мазок наружу — по нему делается отмена.
    /// По умолчанию мазков нет — None
    fn end_stroke(&mut self) -> Option<Stroke> {
        None
    }
    fn name(&self) -> &str { &self.info().name }
    fn description(&self) -> &str { &self.info().description }
}

/// Курсор: позиция мыши + текущий инструмент
pub struct Cursor {
    pub position: (f32, f32),
    tool: Box<dyn Tool>,
}

impl Cursor {
    pub fn new(tool: Box<dyn Tool>) -> Self {
        Self { position: (0.0, 0.0), tool }
    }

    /// Пиксельные координаты курсора
    pub fn pixel_pos(&self) -> (i32, i32) {
        (self.position.0.floor() as i32, self.position.1.floor() as i32)
    }

    /// Применить текущий инструмент
    pub fn click(&mut self, layer: &mut Layer, clip: Option<Rect>) {
        self.tool.use_at(layer, self.position, clip);
    }

    /// Начать мазок текущим инструментом
    pub fn begin_stroke(&mut self) {
        self.tool.begin_stroke();
    }

    /// Закончить мазок и получить его (для отмены)
    pub fn end_stroke(&mut self) -> Option<Stroke> {
        self.tool.end_stroke()
    }

    /// Блок под курсором, если он существует
    pub fn get_block_under_cursor<'a>(&self, layer: &'a Layer) -> Option<&'a GridBlock> {
        let (x, y) = self.pixel_pos();
        layer.block_at(x, y)
    }

    /// Назначить инструмент курсору
    pub fn set_tool(&mut self, tool: Box<dyn Tool>) {
        self.tool = tool;
    }
}