//! ТЕСТОВЫЙ GUI. Всё сделано «в лоб», чтобы проверить ядро (слои, режимы, кривая, форма).
//! Оформление — по макету из Figma: тёмные панели-«пилюли», моноширинный шрифт, капс.

mod canvas;
mod context;
mod history;
mod project;
mod tools;

use std::fmt;

use iced::mouse;
use iced::widget::canvas::{self as cv, Frame, Geometry, LineCap, LineDash, LineJoin, Path, Stroke};
use iced::widget::image::{FilterMethod, Handle};
use iced::widget::{
    button, center, checkbox, column, container, mouse_area, opaque, pick_list, responsive,
    row, scrollable, slider, space, text, text_input, tooltip, Stack,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::time::Instant;

use iced::event::{self, Event};
use iced::window;
use iced::keyboard::{self, key::Code, key::Physical, Modifiers};
use iced::{
    color, Alignment, Background, Border, Color, Element, Font, Length, Point, Rectangle,
    Renderer, Size, Subscription, Theme, Vector,
};

use canvas::{composite, BlendMode, Layer, Pixel, Rect, TRANSPARENT};
use context::Context;
use history::{Dirty, History};
use tools::brush::{Brush, PaintMode};
use tools::fill::Fill;
use tools::curve::BoundaryFunc;
use tools::shape::BrushShape;

/// Холст показывается плитками TILE × TILE. Зачем: iced 0.14 грузит картинки больше 2 МБ
/// в видеопамять асинхронно, и пока грузит — не рисует их. Если весь холст одна большая
/// картинка, на каждое движение мыши он на кадр пропадает (мерцает). Плитка 256² = 256 КБ.
const TILE: u32 = 256;
/// Рамка из соседних пикселей вокруг плитки. При сглаживании (Linear) край плитки смешивается
/// с соседями — пусть это будут настоящие пиксели холста, а не пустота атласа текстур
const TILE_PAD: u32 = 4;

/// Максимальная сторона холста: 4096² × 4 байта = 64 МБ на экранную картинку
const MAX_SIDE: u32 = 4096;

/// Сколько показывается заставка
const SPLASH_SECS: f32 = 2.4;
/// Картинка заставки встроена прямо в exe: include_bytes! читает файл при компиляции
const SPLASH_PNG: &[u8] = include_bytes!("../assets/splash.png");

/// Заготовки размеров на стартовом экране
const PRESETS: [(&str, u32, u32); 6] = [
    ("Тест", 640, 480),
    ("HD", 1280, 720),
    ("Full HD", 1920, 1080),
    ("Квадрат", 1080, 1080),
    ("A4 · 150 dpi", 1240, 1754),
    ("Иконка", 256, 256),
];
const LUT_SIZE: usize = 256;
/// Пределы масштаба и шаг одного «щелчка» колеса / лупы
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 32.0;
const ZOOM_STEP: f32 = 1.25;
const MAX_RADIUS: i32 = 200;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Scapt — тест")
        .theme(App::theme)
        .subscription(App::subscription)
        .default_font(FONT)
        .window_size(Size::new(1280.0, 760.0))
        .run()
}

// ---------- оформление (цвета сняты с макета) ----------

const FONT: Font = Font::with_name("Cascadia Mono");

const BG: Color = color!(0x282723); // фон окна
const PANEL: Color = color!(0x21201c); // заливка панелей
const PANEL_HI: Color = color!(0x2b2a25); // наведение / активная строка
const BORDER: Color = color!(0x57544a); // обводка
const BORDER_HI: Color = color!(0x8c8775); // обводка активного элемента
const GRID: Color = color!(0x34322c); // сетка на графике
const TEXT: Color = color!(0xf2f0ea);
const MUTED: Color = color!(0x908d7d); // подписи, проценты
const ICON: Color = color!(0x77735f);
const ACCENT: Color = color!(0xc9a23a); // тёмно-жёлтый: кривая, ползунки
const DANGER: Color = color!(0xc0533a);

const BAR_H: f32 = 376.0; // высота боковых панелей
const SIDE_W: f32 = 200.0; // ширина панели слоёв

/// Панель с обводкой и скруглением
fn panel_style(radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(PANEL)),
        border: Border { color: BORDER, width: 1.0, radius: radius.into() },
        ..container::Style::default()
    }
}

/// Кнопка с обводкой: круглая для иконок (radius = 999) или скруглённая для текста
fn outline_button(active: bool, radius: f32) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: (active || hovered).then_some(Background::Color(PANEL_HI)),
            text_color: if active { TEXT } else { MUTED },
            border: Border {
                color: if active { BORDER_HI } else { BORDER },
                width: 1.0,
                radius: radius.into(),
            },
            ..button::Style::default()
        }
    }
}

/// Пункт выпадающего меню: без обводки, подсветка при наведении
fn menu_item_style(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then_some(Background::Color(PANEL_HI)),
        text_color: TEXT,
        border: Border { radius: 8.0.into(), ..Border::default() },
        ..button::Style::default()
    }
}

/// Тонкая горизонтальная линия-разделитель
fn divider<'a>() -> Element<'a, Message> {
    container(space().width(Length::Fill).height(1))
        .style(|_| container::background(BORDER))
        .into()
}

/// Мелкая подпись капсом
fn label<'a>(s: &str) -> iced::widget::Text<'a> {
    text(s.to_uppercase()).size(10).color(MUTED)
}

/// Маленькое поле для ввода числа
fn number_input<'a>(value: &str, on_input: impl Fn(String) -> Message + 'a) -> text_input::TextInput<'a, Message> {
    text_input("", value).on_input(on_input).size(10).padding([2, 5]).width(44)
}

/// Цвет в виде "#RRGGBB"
fn to_hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// "#RRGGBB" или "RRGGBB" -> цвет; None, если строка не такая
fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Текстовая кнопка капсом с обводкой
fn text_button<'a>(s: &str, active: bool, msg: Message) -> button::Button<'a, Message> {
    button(text(s.to_uppercase()).size(10).width(Length::Fill).align_x(Alignment::Center))
        .on_press(msg)
        .padding([6, 8])
        .style(outline_button(active, 10.0))
}

/// Круглая кнопка с нарисованной иконкой
fn icon_button<'a>(kind: IconKind, active: bool, size: f32, msg: Message) -> button::Button<'a, Message> {
    let color = if active { TEXT } else { ICON };
    let pad = size * 0.22;
    button(cv::Canvas::new(Icon { kind, color }).width(size - 2.0 * pad).height(size - 2.0 * pad))
        .on_press(msg)
        .padding(pad)
        .width(size)
        .height(size)
        .style(outline_button(active, 999.0))
}

/// Подпись, значение справа и ползунок под ними
fn slider_block<'a>(
    name: &str,
    value: impl Into<Element<'a, Message>>,
    s: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    column![
        row![label(name), space().width(Length::Fill), value.into()].align_y(Alignment::Center),
        s.into(),
    ]
    .spacing(4)
    .into()
}

// ---------- названия режимов наложения для выпадающего списка ----------


impl fmt::Display for BlendMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            BlendMode::Normal => "Обычный",
            BlendMode::Darken => "Затемнение",
            BlendMode::Multiply => "Умножение",
            BlendMode::ColorBurn => "Затемнение основы",
            BlendMode::Lighten => "Замена светлым",
            BlendMode::Screen => "Экран",
            BlendMode::ColorDodge => "Осветление основы",
            BlendMode::Overlay => "Перекрытие",
            BlendMode::SoftLight => "Мягкий свет",
            BlendMode::HardLight => "Жёсткий свет",
            BlendMode::Difference => "Разница",
            BlendMode::Exclusion => "Исключение",
        };
        write!(f, "{name}")
    }
}

// ---------- состояние ----------

/// Какой экран показан: стартовый (новый проект) или сам редактор
#[derive(Debug, Clone, Copy, PartialEq)]
enum Screen {
    Splash, // заставка при запуске
    Start,
    Editor,
}

/// Инструмент на панели слева
#[derive(Debug, Clone, Copy, PartialEq)]
enum ToolKind {
    Brush,
    Eraser,
    Zoom,   // лупа
    Picker, // пипетка
    Fill,   // заливка
    Select, // прямоугольное выделение
    Move,   // перемещение
}

/// Идёт перемещение: всё, что нужно, чтобы двигать кусок слоя и потом записать отмену
struct MoveState {
    layer: usize,
    start: Point,        // где нажали (координаты холста)
    offset: (i32, i32),  // на сколько пикселей сдвинули сейчас
    src: Rect,           // откуда взяли
    original: Layer,     // слой до перемещения — для истории
    lifted: Layer,       // слой без поднятого куска — «подложка», на которую кладём кусок
    float: Vec<((i32, i32), Pixel)>, // поднятые пиксели (только непрозрачные) на старых местах
    had_selection: bool, // было ли выделение (тогда оно едет вместе с куском)
}

/// Чем залить нижний слой нового проекта
#[derive(Debug, Clone, Copy, PartialEq)]
enum NewBackground {
    Transparent,
    White,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CurveMode {
    Formula,
    Points,
}

struct App {
    screen: Screen,
    splash: Handle,                // картинка заставки
    splash_start: Option<Instant>, // когда заставка начала показываться
    has_project: bool, // уже что-то открыто — со стартового экрана можно вернуться назад
    files_open: bool,  // открыто ли меню «Файлы»
    tool: ToolKind,

    // форма «новый проект» на стартовом экране
    new_name: String,
    new_w: String,
    new_h: String,
    new_bg: NewBackground,
    new_error: Option<String>,

    ctx: Context,
    tiles: Vec<Handle>, // плитки экранной картинки, построчно: индекс ty * tiles_x + tx
    rgba: Vec<u8>, // готовая картинка холста; между кадрами пересчитываем только изменённые блоки
    block_buf: Vec<Pixel>, // буфер для сборки одного блока, чтобы не выделять память каждый раз

    // настройки кисти
    color: [u8; 3],
    radius: i32,
    opacity: f32,
    // то, что набрано в полях ввода (может быть недописанным, например "1" на пути к "120")
    radius_text: String,
    rgb_text: [String; 3],
    hex_text: String,

    // кривая жёсткости
    curve_mode: CurveMode,
    formula: String,
    formula_error: Option<String>,
    points: Vec<(f32, f32)>,
    curve: BoundaryFunc, // для графика и превью; кисть получает свою копию
    curve_open: bool,
    brush_open: bool, // открыт ли редактор кисти рядом с панелью инструментов

    // форма кисти из картинки
    shape: Option<BrushShape>,
    shape_path: String,
    shape_error: Option<String>,
    use_hardness: bool,

    // файлы
    status: Option<(String, bool)>, // сообщение под холстом и признак ошибки
    file_name: String,              // имя для диалога сохранения

    // отмена / повтор
    history: History,
    stroke_layer: usize, // в какой слой идёт текущий мазок

    // вид: масштаб и сдвиг холста на экране
    zoom: Option<f32>,   // None — «вписать в окно»
    offset: Vector,      // сдвиг центра холста от центра области, в пикселях экрана
    fit_scale: Cell<f32>, // масштаб «вписать»; считается во view, поэтому Cell (см. canvas_area)
    modifiers: Modifiers, // зажатые Ctrl/Shift/Alt — нужны для Ctrl+колесо и Alt+клик лупой

    // ровность линии
    spacing: f32,   // шаг между отпечатками, доля радиуса (0.12 = 12%)
    smoothing: f32, // 0 — мазок точно за курсором, 0.9 — сильно «догоняет» с запаздыванием
    smooth_pos: Point, // сглаженная точка, к которой ведём мазок

    // заливка
    tolerance: u8,    // допуск: насколько цвет может отличаться и всё ещё залиться
    contiguous: bool, // только связная область вокруг клика

    // вид
    smooth_view: bool, // сглаживать картинку при масштабе (иначе — чёткие пиксели)

    // пипетка
    picking: bool,     // кнопка зажата — цвет берётся на ходу
    pick_merged: bool, // брать цвет со всех видимых слоёв (иначе — только с активного)

    // выделение и перемещение (само выделение хранится в ctx.selection)
    select_anchor: Option<(i32, i32)>, // угол, от которого тянем выделение
    moving: Option<MoveState>,

    // мазок
    drawing: bool,
    mouse: Point,
    hover: bool, // мышь над холстом — показываем обводку кисти
    last_dab: Option<Point>,
}

#[derive(Debug, Clone)]
enum Message {
    // холст
    MouseMoved(Point, bool), // координаты на холсте и «курсор над холстом»
    MousePressed,
    MouseReleased,
    HoverOut, // курсор ушёл с холста (мазок при этом не прерываем)
    // вид
    SetView(f32, Vector), // новый масштаб и сдвиг
    Pan(Vector),
    ZoomBy(f32),          // относительно центра (кнопки, Ctrl+= / Ctrl+-)
    ZoomFit,
    Zoom100,
    // история и клавиши
    Undo,
    Redo,
    Modifiers(Modifiers),
    // заставка
    Tick(Instant),
    SkipSplash,
    // выделение
    SelectAll,
    Deselect,
    DeleteSelection,
    // экраны и меню
    ToggleFilesMenu,
    CloseMenus,
    NewProject, // вернуться на стартовый экран
    BackToEditor,
    NewName(String),
    NewWidth(String),
    NewHeight(String),
    NewPreset(u32, u32),
    NewBg(NewBackground),
    CreateProject,
    // кисть
    SelectTool(ToolKind),
    ColorChanged(usize, u8),
    RadiusChanged(i32),
    OpacityChanged(f32),
    SpacingChanged(f32),
    SmoothingChanged(f32),
    PickMerged(bool),
    ToleranceChanged(u8),
    Contiguous(bool),
    SmoothView(bool),
    RadiusText(String),
    ChannelText(usize, String),
    HexText(String),
    // слои
    AddLayer,
    DeleteLayer,
    ClearLayer,
    SelectLayer(usize),
    ToggleVisible(usize, bool),
    LayerBlend(BlendMode),
    LayerOpacity(f32),
    // форма
    ShapePathChanged(String),
    LoadShape,
    ClearShape,
    UseHardness(bool),
    // кривая
    OpenCurve(bool),
    SetCurveMode(CurveMode),
    FormulaChanged(String),
    AddPoint(f32, f32),
    MovePoint(usize, f32, f32),
    RemovePoint(usize),
    // файлы
    OpenProject,
    SaveProject,
    ImportPng,
    ExportPng,
}

impl App {
    fn new() -> Self {
        let formula = "1 - t^2".to_string();
        let curve = BoundaryFunc::from_formula("Кривая", "", LUT_SIZE, &formula).unwrap();
        // PNG заставки -> RGBA. ::image — крейт image (а не виджет iced с тем же именем)
        let splash = ::image::load_from_memory(SPLASH_PNG).expect("splash.png встроен в программу").to_rgba8();
        let splash = Handle::from_rgba(splash.width(), splash.height(), splash.into_raw());
        let mut app = Self {
            screen: Screen::Splash,
            splash,
            splash_start: None,
            has_project: false,
            files_open: false,
            tool: ToolKind::Brush,
            new_name: "Без имени".to_string(),
            new_w: "1280".to_string(),
            new_h: "720".to_string(),
            new_bg: NewBackground::White,
            new_error: None,
            ctx: Context::new("Без имени", 640, 480, Box::new(placeholder_brush())),
            tiles: Vec::new(),
            rgba: Vec::new(),
            block_buf: Vec::new(),
            color: [200, 40, 40],
            radius: 12,
            opacity: 0.8,
            radius_text: String::new(),
            rgb_text: Default::default(),
            hex_text: String::new(),
            curve_mode: CurveMode::Formula,
            formula,
            formula_error: None,
            points: vec![(0.0, 1.0), (0.5, 0.8), (1.0, 0.0)],
            curve,
            curve_open: false,
            brush_open: true,
            shape: None,
            shape_path: String::new(),
            shape_error: None,
            use_hardness: true,
            drawing: false,
            mouse: Point::ORIGIN,
            hover: false,
            last_dab: None,
            select_anchor: None,
            moving: None,
            history: History::default(),
            spacing: 0.12,
            smoothing: 0.3,
            smooth_pos: Point::ORIGIN,
            picking: false,
            pick_merged: true,
            tolerance: 32,
            contiguous: true,
            smooth_view: true,
            stroke_layer: 0,
            zoom: None,
            offset: Vector::ZERO,
            fit_scale: Cell::new(1.0),
            modifiers: Modifiers::default(),
            status: None,
            file_name: "рисунок".to_string(),
        };
        app.sync_inputs();
        app.ctx.layers[0].name = "Слой 0".to_string();
        app.rebuild_brush();
        app.resize_buffers();
        app
    }

    fn theme(&self) -> Theme {
        Theme::custom(
            "Scapt",
            iced::theme::Palette {
                background: BG,
                text: TEXT,
                primary: ACCENT,
                success: color!(0x8fa33a),
                warning: color!(0xe0a526),
                danger: DANGER,
            },
        )
    }

    // ---------- update ----------

    fn update(&mut self, message: Message) {
        match message {
            Message::MouseMoved(p, inside) => {
                self.mouse = p;
                self.hover = inside;
                if let Some(a) = self.select_anchor {
                    // выделение тянется от угла до курсора и не выходит за холст
                    let b = (p.x.round() as i32, p.y.round() as i32);
                    self.ctx.selection = Some(Rect::from_corners(a, b).intersect(&self.canvas_rect()));
                } else if self.moving.is_some() {
                    self.move_to(p);
                } else if self.picking {
                    self.pick(p);
                } else if self.drawing {
                    // сглаживание: точка мазка проходит только часть пути до курсора.
                    // Дрожание руки гасится, линия плавнее — как «Smoothing» в Фотошопе
                    let k = 1.0 - self.smoothing;
                    self.smooth_pos = self.smooth_pos + (p - self.smooth_pos) * k;
                    self.stroke_to(self.smooth_pos);
                }
            }
            Message::MousePressed => {
                // пипетка: отдельный инструмент или Alt + клик кистью/заливкой (как в Фотошопе)
                let alt_pick = matches!(self.tool, ToolKind::Brush | ToolKind::Fill) && self.modifiers.alt();
                if self.tool == ToolKind::Picker || alt_pick {
                    self.picking = true;
                    self.pick(self.mouse);
                    return;
                }
                if self.tool == ToolKind::Fill {
                    self.fill_at(self.mouse);
                    return;
                }
                if self.tool == ToolKind::Select {
                    // новое выделение начинается с нуля: просто клик — снять выделение
                    let a = (self.mouse.x.round() as i32, self.mouse.y.round() as i32);
                    self.select_anchor = Some(a);
                    self.ctx.selection = None;
                    return;
                }
                if self.tool == ToolKind::Move {
                    self.begin_move(self.mouse, self.modifiers.alt());
                    return;
                }
                self.drawing = true;
                self.last_dab = None;
                self.smooth_pos = self.mouse;
                self.stroke_layer = self.ctx.active_layer;
                self.ctx.begin_stroke();
                self.stroke_to(self.mouse);
            }
            Message::MouseReleased => {
                self.picking = false;
                if self.select_anchor.take().is_some() {
                    // нулевое выделение (просто клик) — значит «без выделения»
                    if self.ctx.selection.is_some_and(|r| r.is_empty()) {
                        self.ctx.selection = None;
                    }
                }
                self.finish_move();
                // сглаженная точка отстаёт от курсора — доводим мазок до места, где отпустили
                if self.drawing && self.smoothing > 0.0 {
                    self.stroke_to(self.mouse);
                }
                self.end_stroke();
            }
            Message::SpacingChanged(v) => self.spacing = v,
            Message::SmoothingChanged(v) => self.smoothing = v,
            Message::PickMerged(v) => self.pick_merged = v,
            Message::ToleranceChanged(v) => {
                self.tolerance = v;
                self.rebuild_brush();
            }
            Message::Contiguous(v) => {
                self.contiguous = v;
                self.rebuild_brush();
            }
            Message::SmoothView(v) => self.smooth_view = v,
            Message::HoverOut => self.hover = false,
            Message::SetView(zoom, offset) => {
                self.zoom = Some(zoom);
                self.offset = offset;
            }
            Message::Pan(d) => self.offset = self.offset + d,
            Message::ZoomBy(factor) => {
                // относительно центра области: сдвиг масштабируется вместе с холстом
                let z = self.current_zoom();
                let nz = (z * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                self.offset = self.offset * (nz / z);
                self.zoom = Some(nz);
            }
            Message::ZoomFit => {
                self.zoom = None;
                self.offset = Vector::ZERO;
            }
            Message::Zoom100 => {
                self.zoom = Some(1.0);
                self.offset = Vector::ZERO;
            }
            Message::Undo | Message::Redo => {
                // во время мазка/перемещения и вне редактора историю не трогаем
                if self.drawing || self.moving.is_some() || self.screen != Screen::Editor {
                    return;
                }
                let dirty = match message {
                    Message::Undo => self.history.undo(&mut self.ctx),
                    _ => self.history.redo(&mut self.ctx),
                };
                match dirty {
                    Some(Dirty::Rect(x0, y0, x1, y1)) => self.render_rect(x0, y0, x1, y1),
                    Some(Dirty::All) => self.render(),
                    None => {}
                }
            }
            Message::Modifiers(m) => self.modifiers = m,
            Message::Tick(now) => {
                if self.screen == Screen::Splash {
                    let start = *self.splash_start.get_or_insert(now);
                    if now.duration_since(start).as_secs_f32() >= SPLASH_SECS {
                        self.screen = Screen::Start;
                    }
                }
            }
            Message::SkipSplash => self.screen = Screen::Start,
            Message::SelectAll => {
                if self.screen == Screen::Editor {
                    self.ctx.selection = Some(self.canvas_rect());
                }
            }
            Message::Deselect => self.ctx.selection = None,
            Message::DeleteSelection => self.delete_selection(),
            Message::ToggleFilesMenu => self.files_open = !self.files_open,
            Message::CloseMenus => self.files_open = false,
            Message::NewProject => {
                self.files_open = false;
                self.new_error = None;
                self.screen = Screen::Start;
            }
            Message::BackToEditor => self.screen = Screen::Editor,
            Message::NewName(s) => self.new_name = s,
            Message::NewWidth(s) => self.new_w = s,
            Message::NewHeight(s) => self.new_h = s,
            Message::NewPreset(w, h) => {
                self.new_w = w.to_string();
                self.new_h = h.to_string();
            }
            Message::NewBg(bg) => self.new_bg = bg,
            Message::CreateProject => self.create_project(),
            // повторный клик по выбранному инструменту прячет/показывает его настройки
            Message::SelectTool(kind) => {
                if self.tool == kind {
                    self.brush_open = !self.brush_open;
                } else {
                    self.tool = kind;
                    self.brush_open = true;
                    self.rebuild_brush();
                }
            }
            Message::ColorChanged(i, v) => {
                self.color[i] = v;
                self.sync_inputs();
                self.rebuild_brush();
            }
            Message::RadiusChanged(r) => {
                self.radius = r;
                self.sync_inputs();
                self.rebuild_brush();
            }
            // в поле может быть недописанное число — применяем, только если оно разбирается,
            // и сам текст поля не трогаем, чтобы не мешать печатать
            Message::RadiusText(s) => {
                if let Ok(r) = s.trim().parse::<i32>() {
                    self.radius = r.clamp(1, MAX_RADIUS);
                    self.rebuild_brush();
                }
                self.radius_text = s;
            }
            Message::ChannelText(i, s) => {
                if let Ok(v) = s.trim().parse::<u16>() {
                    self.color[i] = v.min(255) as u8;
                    self.hex_text = to_hex(self.color);
                    self.rebuild_brush();
                }
                self.rgb_text[i] = s;
            }
            Message::HexText(s) => {
                if let Some(c) = parse_hex(&s) {
                    self.color = c;
                    self.rgb_text = c.map(|v| v.to_string());
                    self.rebuild_brush();
                }
                self.hex_text = s;
            }
            Message::OpacityChanged(o) => {
                self.opacity = o;
                self.rebuild_brush();
            }
            Message::AddLayer => self.layers_op(|ctx| {
                let name = format!("Слой {}", ctx.layers.len());
                ctx.add_layer(&name);
            }),
            Message::DeleteLayer => {
                // последний слой не удаляем: у проекта всегда есть хотя бы один
                if self.ctx.layers.len() > 1 {
                    self.layers_op(|ctx| {
                        ctx.layers.remove(ctx.active_layer);
                        ctx.active_layer = ctx.active_layer.min(ctx.layers.len() - 1);
                    });
                }
            }
            Message::ClearLayer => self.layers_op(|ctx| {
                let i = ctx.active_layer;
                let old = &ctx.layers[i];
                let mut fresh = canvas::Layer::new(&old.name, ctx.block_size);
                fresh.blend = old.blend;
                fresh.opacity = old.opacity;
                fresh.visible = old.visible;
                ctx.layers[i] = fresh;
            }),
            Message::SelectLayer(i) => self.ctx.select_layer(i),
            Message::ToggleVisible(i, v) => {
                self.ctx.layers[i].visible = v;
                self.render();
            }
            Message::LayerBlend(mode) => {
                let i = self.ctx.active_layer;
                self.ctx.layers[i].blend = mode;
                self.render();
            }
            Message::LayerOpacity(o) => {
                let i = self.ctx.active_layer;
                self.ctx.layers[i].opacity = o;
                self.render();
            }
            Message::ShapePathChanged(p) => self.shape_path = p,
            Message::LoadShape => {
                // «Копировать как путь» в Проводнике добавляет кавычки — убираем
                let path = self.shape_path.trim().trim_matches('"');
                match BrushShape::from_image(path) {
                    Ok(shape) => {
                        self.shape = Some(shape);
                        self.shape_error = None;
                    }
                    Err(e) => self.shape_error = Some(e),
                }
                self.rebuild_brush();
            }
            Message::ClearShape => {
                self.shape = None;
                self.shape_error = None;
                self.rebuild_brush();
            }
            Message::UseHardness(on) => {
                self.use_hardness = on;
                self.rebuild_brush();
            }
            Message::OpenCurve(open) => self.curve_open = open,
            Message::SetCurveMode(mode) => {
                self.curve_mode = mode;
                self.rebuild_brush();
            }
            Message::FormulaChanged(f) => {
                self.formula = f;
                self.rebuild_brush();
            }
            Message::AddPoint(t, a) => {
                self.points.push((t, a));
                self.rebuild_brush();
            }
            Message::MovePoint(i, t, a) => {
                self.points[i] = (t, a);
                self.rebuild_brush();
            }
            Message::RemovePoint(i) => {
                if self.points.len() > 2 {
                    self.points.remove(i);
                    self.rebuild_brush();
                }
            }
            // Диалоги rfd «блокирующие»: update ждёт, пока окно выбора файла закроют.
            // Для тестового GUI это нормально; позже можно перейти на AsyncFileDialog + Task.
            Message::SaveProject => {
                self.files_open = false;
                let dialog = rfd::FileDialog::new()
                    .add_filter("Проект Scapt", &["scapt"])
                    .set_file_name(format!("{}.scapt", self.file_name));
                if let Some(path) = dialog.save_file() {
                    let path = path.to_string_lossy();
                    let result = project::save_project(&self.ctx, &path);
                    self.report(result, &path, "сохранено");
                }
            }
            Message::OpenProject => {
                self.files_open = false;
                if let Some(path) = rfd::FileDialog::new().add_filter("Проект Scapt", &["scapt"]).pick_file() {
                    let path = path.to_string_lossy();
                    let result = project::load_project(&mut self.ctx, &path);
                    let ok = result.is_ok();
                    self.report(result, &path, "открыто");
                    if ok {
                        self.screen = Screen::Editor;
                        self.has_project = true;
                        self.resize_buffers();
                    } else if self.screen == Screen::Start {
                        // на стартовом экране строки статуса нет — показываем ошибку в форме
                        self.new_error = self.status.as_ref().map(|(m, _)| m.clone());
                    }
                }
            }
            Message::ExportPng => {
                self.files_open = false;
                let dialog = rfd::FileDialog::new()
                    .add_filter("PNG", &["png"])
                    .set_file_name(format!("{}.png", self.file_name));
                if let Some(path) = dialog.save_file() {
                    let path = path.to_string_lossy();
                    let result = project::export_png(&self.ctx, &path);
                    self.report(result, &path, "экспортировано");
                }
            }
            Message::ImportPng => {
                self.files_open = false;
                if let Some(path) = rfd::FileDialog::new().add_filter("Картинки", &["png", "jpg", "jpeg"]).pick_file() {
                    let path = path.to_string_lossy().into_owned();
                    let mut result = Ok(());
                    self.layers_op(|ctx| result = project::import_png(ctx, &path));
                    self.report(result, &path, "импортировано");
                }
            }
        }
    }

    /// Создать проект по форме стартового экрана
    fn create_project(&mut self) {
        let parse = |s: &str| s.trim().parse::<u32>().ok().filter(|v| (1..=MAX_SIDE).contains(v));
        let (Some(w), Some(h)) = (parse(&self.new_w), parse(&self.new_h)) else {
            self.new_error = Some(format!("размер — целые числа от 1 до {MAX_SIDE}"));
            return;
        };
        let name = match self.new_name.trim() {
            "" => "Без имени".to_string(),
            n => n.to_string(),
        };

        self.ctx = Context::new(&name, w, h, Box::new(placeholder_brush()));
        self.ctx.layers[0].name = "Слой 0".to_string();
        if self.new_bg == NewBackground::White {
            let layer = &mut self.ctx.layers[0];
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    layer.set_pixel(x, y, [255, 255, 255, 255]);
                }
            }
        }
        self.rebuild_brush(); // в новый Context — наша настоящая кисть
        self.file_name = name;
        self.status = None;
        self.new_error = None;
        self.has_project = true;
        self.screen = Screen::Editor;
        self.resize_buffers();
    }

    /// Холст поменял размер (новый проект / открыт файл): пересоздать картинку,
    /// забыть историю старого проекта и вписать холст в окно
    fn resize_buffers(&mut self) {
        self.history.clear();
        self.ctx.selection = None;
        self.moving = None;
        self.zoom = None;
        self.offset = Vector::ZERO;
        self.rebuild_brush(); // заливке нужен новый размер холста
        self.rgba = vec![0u8; (self.ctx.width * self.ctx.height * 4) as usize];
        self.render();
    }

    /// Показать под холстом результат операции с файлом; при успехе запомнить имя файла
    fn report(&mut self, result: Result<(), String>, path: &str, done: &str) {
        self.status = Some(match result {
            Ok(()) => {
                if let Some(stem) = std::path::Path::new(path).file_stem() {
                    self.file_name = stem.to_string_lossy().into_owned();
                }
                (format!("{done}: {path}"), false)
            }
            Err(e) => (e, true),
        });
    }

    /// Ставит отпечатки кисти от прошлого отпечатка до p ровно через каждые step пикселей.
    /// Остаток (если до p не хватило на целый шаг) переносится на следующее движение мыши:
    /// отпечатки ложатся с одинаковым шагом, и край мазка ровный.
    /// Потом пересчитывает на экране только прямоугольник, который задели отпечатки.
    fn stroke_to(&mut self, p: Point) {
        let step = (self.radius as f32 * self.spacing).max(0.5);
        let mut dabs = Vec::new();
        match self.last_dab {
            None => dabs.push(p),
            Some(last) => {
                let dist = last.distance(p);
                if dist < step {
                    return; // ждём, пока мышь уйдёт на целый шаг
                }
                let dir = (p - last) * (1.0 / dist); // единичный вектор направления
                let mut t = step;
                while t <= dist {
                    dabs.push(last + dir * t);
                    t += step;
                }
            }
        }

        // общий прямоугольник всех отпечатков: (x0, y0) включительно, (x1, y1) не включительно
        let r = self.radius + 1; // +1 — полпикселя сглаживания края и дробный центр
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &q in &dabs {
            self.dab(q);
            let (cx, cy) = (q.x.floor() as i32, q.y.floor() as i32);
            x0 = x0.min(cx - r);
            y0 = y0.min(cy - r);
            x1 = x1.max(cx + r + 1);
            y1 = y1.max(cy + r + 1);
        }
        self.render_rect(x0, y0, x1, y1);
    }

    /// Весь холст как прямоугольник
    fn canvas_rect(&self) -> Rect {
        Rect { x0: 0, y0: 0, x1: self.ctx.width as i32, y1: self.ctx.height as i32 }
    }

    /// Начать перемещение: поднять выделенный кусок активного слоя (или весь холст).
    /// copy = true (Alt) — на старом месте ничего не стираем, получится копия
    fn begin_move(&mut self, p: Point, copy: bool) {
        let li = self.ctx.active_layer;
        let src = self.ctx.selection.unwrap_or(self.canvas_rect());
        let original = self.ctx.layers[li].clone();
        let mut lifted = original.clone();
        let mut float = Vec::new();
        for (x, y) in src.pixels() {
            let px = original.get_pixel(x, y);
            if px[3] > 0 {
                float.push(((x, y), px));
                if !copy {
                    lifted.set_pixel(x, y, TRANSPARENT);
                }
            }
        }
        self.moving = Some(MoveState {
            layer: li,
            start: p,
            offset: (0, 0),
            src,
            original,
            lifted,
            float,
            had_selection: self.ctx.selection.is_some(),
        });
    }

    /// Курсор сдвинулся во время перемещения: переложить кусок на новое место
    fn move_to(&mut self, p: Point) {
        // отдельный блок: пока жив ms (заимствование self.moving), render_rect(&mut self) вызвать нельзя
        let dirty = {
            let Some(ms) = self.moving.as_mut() else { return };
            let off = ((p.x - ms.start.x).round() as i32, (p.y - ms.start.y).round() as i32);
            if off == ms.offset {
                return;
            }
            let layer = &mut self.ctx.layers[ms.layer];
            // Слой отличается от «подложки» только там, где сейчас лежит кусок, —
            // возвращаем эти пиксели из подложки, а не весь прямоугольник
            for &((x, y), _) in &ms.float {
                let (ox, oy) = (x + ms.offset.0, y + ms.offset.1);
                layer.set_pixel(ox, oy, ms.lifted.get_pixel(ox, oy));
            }
            // кладём кусок на новое место поверх подложки
            for &((x, y), px) in &ms.float {
                let (nx, ny) = (x + off.0, y + off.1);
                let base = ms.lifted.get_pixel(nx, ny);
                layer.set_pixel(nx, ny, composite(base, px, BlendMode::Normal, 1.0));
            }
            let old = ms.src.offset(ms.offset.0, ms.offset.1);
            let new = ms.src.offset(off.0, off.1);
            ms.offset = off;
            if ms.had_selection {
                self.ctx.selection = Some(new); // выделение едет вместе с куском
            }
            old.union(&new)
        };
        self.render_rect(dirty.x0, dirty.y0, dirty.x1, dirty.y1);
    }

    /// Отпустили кнопку: перемещение закончено — одна запись в историю
    fn finish_move(&mut self) {
        let Some(ms) = self.moving.take() else { return };
        if ms.offset == (0, 0) {
            return; // не сдвинули — ничего не изменилось
        }
        let layer = &self.ctx.layers[ms.layer];
        // изменились только старые и новые места куска; HashMap убирает повторы
        let mut changes = HashMap::new();
        for &((x, y), _) in &ms.float {
            for pos in [(x, y), (x + ms.offset.0, y + ms.offset.1)] {
                let (before, after) = (ms.original.get_pixel(pos.0, pos.1), layer.get_pixel(pos.0, pos.1));
                if before != after {
                    changes.insert(pos, (before, after));
                }
            }
        }
        let changes = changes.into_iter().map(|(pos, (b, a))| (pos, b, a)).collect();
        self.history.push(history::Action::Pixels { layer: ms.layer, changes });
    }

    /// Delete: стереть содержимое выделения на активном слое
    fn delete_selection(&mut self) {
        let Some(sel) = self.ctx.selection else { return };
        if self.screen != Screen::Editor || self.drawing || self.moving.is_some() {
            return;
        }
        let li = self.ctx.active_layer;
        let layer = &mut self.ctx.layers[li];
        let mut changes = Vec::new();
        for (x, y) in sel.pixels() {
            let before = layer.get_pixel(x, y);
            if before[3] > 0 {
                layer.set_pixel(x, y, TRANSPARENT);
                changes.push(((x, y), before, TRANSPARENT));
            }
        }
        if !changes.is_empty() {
            self.history.push(history::Action::Pixels { layer: li, changes });
            self.render_rect(sel.x0, sel.y0, sel.x1, sel.y1);
        }
    }

    /// Заливка в точке p: тот же путь, что у мазка (begin → click → end),
    /// поэтому отмена работает так же
    fn fill_at(&mut self, p: Point) {
        self.stroke_layer = self.ctx.active_layer;
        self.ctx.begin_stroke();
        self.ctx.move_cursor(p.x, p.y);
        self.ctx.click();
        let Some(stroke) = self.ctx.end_stroke() else { return };
        if stroke.before.is_empty() {
            return; // ничего не изменилось (клик вне холста или тот же цвет)
        }
        // перерисовать только изменённый прямоугольник
        let keys = stroke.before.keys();
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &(x, y) in keys {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
        self.history.push_stroke(&self.ctx, self.stroke_layer, stroke);
        self.render_rect(x0, y0, x1, y1);
    }

    /// Пипетка: взять цвет в точке p холста
    fn pick(&mut self, p: Point) {
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        if x < 0 || y < 0 || x >= self.ctx.width as i32 || y >= self.ctx.height as i32 {
            return;
        }
        let px = if self.pick_merged {
            self.ctx.composite_pixel(x, y)
        } else {
            self.ctx.layers[self.ctx.active_layer].get_pixel(x, y)
        };
        if px[3] == 0 {
            return; // прозрачное место — цвета там нет
        }
        // в слоях цвет умножен на альфу — для кисти нужен «чистый»
        let [r, g, b, _] = project::unpremultiply(px);
        self.color = [r, g, b];
        self.sync_inputs();
        self.rebuild_brush();
    }

    fn end_stroke(&mut self) {
        if !self.drawing {
            return;
        }
        self.drawing = false;
        self.last_dab = None;
        if let Some(stroke) = self.ctx.end_stroke() {
            self.history.push_stroke(&self.ctx, self.stroke_layer, stroke);
        }
    }

    /// Операция со слоями с записью в историю: снимок до, сама операция, снимок после.
    /// op — замыкание, которое меняет Context (как функция-аргумент в Python)
    fn layers_op(&mut self, op: impl FnOnce(&mut Context)) {
        let before = (self.ctx.layers.clone(), self.ctx.active_layer);
        op(&mut self.ctx);
        let after = (self.ctx.layers.clone(), self.ctx.active_layer);
        self.history.push(history::Action::Layers { before, after });
        self.render();
    }

    /// Горячие клавиши. Слушаем все события окна, но берём только нужные
    fn subscription(&self) -> Subscription<Message> {
        let events = event::listen_with(|event, status, _window| match event {
            // Ctrl/Shift/Alt нужны всегда, даже если событие забрал виджет
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => Some(Message::Modifiers(m)),
            // мазок могли закончить над панелью: opaque забирает событие себе, и холст
            // его не увидит. Подписка видит все события — заканчиваем мазок отсюда.
            // Лишний MouseReleased безопасен: end_stroke проверяет, идёт ли мазок
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::MouseReleased),
            // остальное — только если клавишу не забрало поле ввода
            Event::Keyboard(keyboard::Event::KeyPressed { physical_key, modifiers, .. })
                if status == event::Status::Ignored =>
            {
                shortcut(physical_key, modifiers)
            }
            _ => None,
        });
        if self.screen == Screen::Splash {
            // таймера в нашей сборке iced нет (он есть с tokio/smol), зато есть событие на
            // каждый кадр. Подписываемся только на время заставки — потом процессор не грузим
            Subscription::batch([events, window::frames().map(Message::Tick)])
        } else {
            events
        }
    }

    /// Масштаб и левый верхний угол холста в области размера size
    fn view_transform(&self, size: Size) -> (f32, Point) {
        let (w, h) = (self.ctx.width as f32, self.ctx.height as f32);
        let z = self.current_zoom();
        let center = self.free_area(size).center();
        // угол холста — на целом пикселе экрана: при 100% пиксель холста = пиксель экрана, без размытия
        let origin = Point::new(
            (center.x + self.offset.x - w * z / 2.0).round(),
            (center.y + self.offset.y - h * z / 2.0).round(),
        );
        (z, origin)
    }

    /// Новый вид после масштабирования в factor раз так, чтобы точка холста
    /// под anchor (координаты в области) осталась под курсором
    fn zoom_at(&self, size: Size, anchor: Point, factor: f32) -> Message {
        let (z, origin) = self.view_transform(size);
        let nz = (z * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        // точка холста под курсором
        let c = (anchor - origin) * (1.0 / z);
        // где должен оказаться угол холста, чтобы c осталась под курсором
        let new_origin = anchor - c * nz;
        let (w, h) = (self.ctx.width as f32, self.ctx.height as f32);
        let center = self.free_area(size).center();
        let offset = Vector::new(
            new_origin.x + w * nz / 2.0 - center.x,
            new_origin.y + h * nz / 2.0 - center.y,
        );
        Message::SetView(nz, offset)
    }

    /// Переписать поля ввода из текущих значений (после ползунков)
    fn sync_inputs(&mut self) {
        self.radius_text = self.radius.to_string();
        self.rgb_text = self.color.map(|v| v.to_string());
        self.hex_text = to_hex(self.color);
    }

    fn dab(&mut self, p: Point) {
        self.ctx.move_cursor(p.x, p.y);
        self.ctx.click();
        self.last_dab = Some(p);
    }

    /// Строит кривую из текущего источника (формула или точки)
    fn build_curve(&self) -> Result<BoundaryFunc, String> {
        match self.curve_mode {
            CurveMode::Formula => BoundaryFunc::from_formula("Кривая", "", LUT_SIZE, &self.formula),
            CurveMode::Points => Ok(BoundaryFunc::from_points("Кривая", "", LUT_SIZE, self.points.clone())),
        }
    }

    /// BoundaryFunc не умеет Clone, поэтому кривую строим дважды: для кисти и для графика
    fn rebuild_brush(&mut self) {
        match (self.build_curve(), self.build_curve()) {
            (Ok(for_brush), Ok(for_plot)) => {
                self.formula_error = None;
                self.curve = for_plot;
                let [r, g, b] = self.color;
                // у заливки своя структура — тот же трейт Tool, другая реализация
                if self.tool == ToolKind::Fill {
                    let bounds = (self.ctx.width, self.ctx.height);
                    let fill = Fill::new([r, g, b, 255], self.opacity, self.tolerance, self.contiguous, bounds);
                    self.ctx.set_tool(Box::new(fill));
                    return;
                }
                // as_ref: заглянуть в Option, не забирая форму из self
                let mask = self.shape.as_ref().map(|s| s.to_mask(self.radius));
                let brush = Brush::new(
                    "Кисть", "Рисование", "Тестовая кисть",
                    [r, g, b, 255], self.radius, self.opacity, for_brush, mask,
                );
                if let Ok(mut brush) = brush {
                    brush.set_use_hardness(self.use_hardness);
                    brush.set_mode(match self.tool {
                        ToolKind::Eraser => PaintMode::Erase,
                        // лупа и пипетка не рисуют, режим им не важен
                        _ => PaintMode::Paint,
                    });
                    self.ctx.set_tool(Box::new(brush));
                }
            }
            (Err(e), _) | (_, Err(e)) => self.formula_error = Some(e),
        }
    }

    /// Пересобрать весь холст (смена режима, видимости, нового слоя...)
    fn render(&mut self) {
        self.render_rect(0, 0, self.ctx.width as i32, self.ctx.height as i32);
    }

    /// Пересобрать прямоугольник [x0, x1) × [y0, y1) поверх шахматки.
    /// Работаем целыми блоками: все блоки, которые задевает прямоугольник,
    /// собираются через Context::composite_block и копируются в картинку.
    fn render_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let (w, h) = (self.ctx.width as i32, self.ctx.height as i32);
        // обрезаем по краям холста: кисть могла выйти за них
        let (x0, y0) = (x0.max(0), y0.max(0));
        let (x1, y1) = (x1.min(w), y1.min(h));
        if x0 >= x1 || y0 >= y1 {
            return;
        }

        let bs = self.ctx.block_size as i32;
        // диапазон блоков, включительно: от блока с (x0, y0) до блока с (x1 - 1, y1 - 1)
        for by in y0.div_euclid(bs)..=(y1 - 1).div_euclid(bs) {
            for bx in x0.div_euclid(bs)..=(x1 - 1).div_euclid(bs) {
                // разные поля self — заимствования не конфликтуют
                self.ctx.composite_block((bx, by), &mut self.block_buf);

                // часть блока, которая попадает на холст
                let (gx0, gy0) = ((bx * bs).max(0), (by * bs).max(0));
                let (gx1, gy1) = (((bx + 1) * bs).min(w), ((by + 1) * bs).min(h));
                for y in gy0..gy1 {
                    for x in gx0..gx1 {
                        let [r, g, b, a] = self.block_buf[((y - by * bs) * bs + (x - bx * bs)) as usize];
                        let checker = if (x / 8 + y / 8) % 2 == 0 { 70.0 } else { 50.0 };
                        let k = 1.0 - a as f32 / 255.0;
                        // цвета premultiplied, поэтому фон просто добавляется с весом (1 - a)
                        let i = ((y * w + x) * 4) as usize;
                        for (c, v) in [r, g, b].into_iter().enumerate() {
                            self.rgba[i + c] = (v as f32 + checker * k).round().min(255.0) as u8;
                        }
                        self.rgba[i + 3] = 255;
                    }
                }
            }
        }
        self.update_tiles(x0, y0, x1, y1);
    }

    /// Сколько плиток по горизонтали и вертикали
    fn tiles_count(&self) -> (u32, u32) {
        (self.ctx.width.div_ceil(TILE), self.ctx.height.div_ceil(TILE))
    }

    /// Прямоугольник холста, из которого сделана текстура плитки: сама плитка + рамка TILE_PAD
    /// (у края холста рамки нет). Возвращает (x, y, ширина, высота)
    fn tile_source(&self, tx: u32, ty: u32) -> (u32, u32, u32, u32) {
        let (w, h) = (self.ctx.width, self.ctx.height);
        let (px, py) = (tx * TILE, ty * TILE);
        let (x0, y0) = (px.saturating_sub(TILE_PAD), py.saturating_sub(TILE_PAD));
        let (x1, y1) = ((px + TILE + TILE_PAD).min(w), (py + TILE + TILE_PAD).min(h));
        (x0, y0, x1 - x0, y1 - y0)
    }

    /// Пересоздать плитки, которые задевает прямоугольник [x0, x1) × [y0, y1).
    /// Из-за рамки пиксель у края плитки есть и в текстуре соседки — расширяем на TILE_PAD
    fn update_tiles(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let (w, h) = (self.ctx.width, self.ctx.height);
        let (tiles_x, tiles_y) = self.tiles_count();
        if self.tiles.len() != (tiles_x * tiles_y) as usize {
            // размер холста поменялся: заготовки, все плитки ниже перезапишутся
            self.tiles = vec![Handle::from_rgba(1, 1, vec![0u8; 4]); (tiles_x * tiles_y) as usize];
        }
        let (t, pad) = (TILE as i32, TILE_PAD as i32);
        let (x0, y0) = ((x0 - pad).max(0), (y0 - pad).max(0));
        let (x1, y1) = ((x1 + pad).min(w as i32), (y1 + pad).min(h as i32));
        for ty in (y0 / t) as u32..=((y1 - 1) / t) as u32 {
            for tx in (x0 / t) as u32..=((x1 - 1) / t) as u32 {
                let (sx, sy, sw, sh) = self.tile_source(tx, ty);
                let mut bytes = Vec::with_capacity((sw * sh * 4) as usize);
                for y in sy..sy + sh {
                    let row = ((y * w + sx) * 4) as usize;
                    bytes.extend_from_slice(&self.rgba[row..row + (sw * 4) as usize]);
                }
                self.tiles[(ty * tiles_x + tx) as usize] = Handle::from_rgba(sw, sh, bytes);
            }
        }
    }

    // ---------- view ----------

    fn view(&self) -> Element<'_, Message> {
        match self.screen {
            Screen::Splash => self.splash_view(),
            Screen::Start => self.start_view(),
            Screen::Editor => self.editor_view(),
        }
    }

    fn editor_view(&self) -> Element<'_, Message> {
        // кнопка инструмента с подсказкой справа при наведении
        let tool = |kind, icon, hint: &'static str| {
            tooltip(
                icon_button(icon, self.tool == kind, 26.0, Message::SelectTool(kind)),
                container(text(hint.to_uppercase()).size(10).color(TEXT))
                    .padding([4, 8])
                    .style(panel_style(8.0)),
                tooltip::Position::Right,
            )
            .gap(8)
        };

        let toolbar = container(
            column![
                tool(ToolKind::Brush, IconKind::Brush, "Кисть"),
                tool(ToolKind::Eraser, IconKind::Eraser, "Ластик"),
                tool(ToolKind::Zoom, IconKind::Zoom, "Лупа"),
                tool(ToolKind::Fill, IconKind::Fill, "Заливка"),
                tool(ToolKind::Select, IconKind::Select, "Выделение (Ctrl+A / Ctrl+D)"),
                tool(ToolKind::Move, IconKind::Move, "Перемещение (Alt — копия)"),
                tool(ToolKind::Picker, IconKind::Picker, "Пипетка (у кисти — Alt + клик)"),
            ]
            .spacing(6),
        )
        .padding(4)
        .width(36)
        .height(BAR_H)
        .align_x(Alignment::Center) // кнопки по центру узкой панели
        .style(panel_style(18.0));

        // Каждая панель — opaque: клик по ней не проходит на холст под ней.
        // Пустое место вокруг панелей прозрачно — и для глаз, и для мыши.
        let mut left = row![opaque(toolbar)].spacing(10).align_y(Alignment::Center);
        if self.brush_open {
            left = left.push(opaque(match self.tool {
                ToolKind::Zoom => self.zoom_panel(),
                ToolKind::Picker => self.picker_panel(),
                ToolKind::Fill => self.fill_panel(),
                ToolKind::Select => self.select_panel(),
                ToolKind::Move => self.move_panel(),
                ToolKind::Brush | ToolKind::Eraser => self.brush_panel(),
            }));
        }

        // справа: слои + полоса с кнопками
        let right = column![opaque(self.layers_panel()), opaque(self.layers_bar())].spacing(12);

        let panels = column![
            row![opaque(self.top_bar())],
            row![
                container(left).center_y(Length::Fill),
                space().width(Length::Fill),
                container(right).center_y(Length::Fill),
            ]
            .height(Length::Fill),
        ]
        .spacing(10)
        .padding(9);

        // нижний слой — холст на всё окно (responsive сообщает размер окна), панели плавают сверху
        let canvas = responsive(move |size| self.canvas_area(size));

        let mut layers: Vec<Element<'_, Message>> = vec![canvas.into(), panels.into()];
        if self.curve_open {
            let backdrop = container(center(self.curve_editor()))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_| container::background(Color::from_rgba8(0, 0, 0, 0.55)));
            layers.push(opaque(backdrop));
        }
        if self.files_open {
            layers.push(self.files_menu());
        }
        Stack::with_children(layers).into()
    }

    /// Верхняя полоса: кнопка «Файлы» и имя проекта
    fn top_bar(&self) -> Element<'_, Message> {
        container(
            row![
                text_button("Файлы", self.files_open, Message::ToggleFilesMenu).width(90),
                text(self.file_name.to_uppercase()).size(11).color(TEXT),
                text(format!("{} × {}", self.ctx.width, self.ctx.height)).size(10).color(MUTED),
                // клик по масштабу — вписать в окно
                button(text(format!("{}%", (self.current_zoom() * 100.0).round())).size(10).color(MUTED))
                    .on_press(Message::ZoomFit)
                    .padding([2, 6])
                    .style(menu_item_style),
                self.status_line(),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([0, 12])
        .center_y(36)
        .style(panel_style(18.0))
        .into()
    }

    /// Выпадающее меню «Файлы». Под ним — прозрачная подложка на всё окно:
    /// клик мимо меню закрывает его.
    fn files_menu(&self) -> Element<'_, Message> {
        let item = |name: &str, msg| {
            button(text(name.to_uppercase()).size(10))
                .on_press(msg)
                .width(Length::Fill)
                .padding([7, 10])
                .style(menu_item_style)
        };

        let menu = container(
            column![
                item("Новый проект…", Message::NewProject),
                item("Открыть…", Message::OpenProject),
                divider(),
                item("Сохранить…", Message::SaveProject),
                divider(),
                item("Импорт PNG как слой…", Message::ImportPng),
                item("Экспорт в PNG…", Message::ExportPng),
            ]
            .spacing(3),
        )
        .padding(6)
        .width(240)
        .style(panel_style(12.0));

        // меню прямо под кнопкой: отступ окна 9 + полоса 36 + зазор 6
        let placed = column![space().height(51), row![space().width(9), menu]];
        opaque(
            mouse_area(container(placed).width(Length::Fill).height(Length::Fill))
                .on_press(Message::CloseMenus),
        )
    }

    /// Холст по центру свободного места, уменьшенный, если не помещается
    /// Область холста: сам холст (масштаб, сдвиг, мышь — внутри CanvasView) и строка под ним
    fn canvas_area(&self, size: Size) -> Element<'_, Message> {
        let free = self.free_area(size);
        let (w, h) = (self.ctx.width as f32, self.ctx.height as f32);
        // масштаб «вписать в окно» зависит от размера окна, а он известен только здесь, во view.
        // view получает &self и менять поля не может — кроме Cell: это «ячейка», которую можно
        // перезаписать и через &self (interior mutability). update потом читает её для Ctrl+= / Ctrl+-
        self.fit_scale.set((free.width / w).min(free.height / h).clamp(MIN_ZOOM, 1.0));

        // холст и поверх него — рамка выделения и обводка кисти (каждый слой Stack рисуется отдельно)
        Stack::with_children([
            cv::Canvas::new(CanvasView { app: self }).width(Length::Fill).height(Length::Fill).into(),
            cv::Canvas::new(Overlay { app: self }).width(Length::Fill).height(Length::Fill).into(),
        ])
        .into()
    }

    /// Текущий масштаб (с учётом «вписать»)
    fn current_zoom(&self) -> f32 {
        self.zoom.unwrap_or(self.fit_scale.get())
    }

    /// Часть окна, не занятая панелями: в неё холст вписывается и вокруг её центра стоит.
    /// Числа — размеры панелей из view (отступы 9, зазоры 10, ширины панелей) плюс поля 12
    fn free_area(&self, size: Size) -> Rectangle {
        let left = 9.0 + 36.0 + 10.0 + if self.brush_open { 250.0 + 10.0 } else { 0.0 } + 12.0;
        let right = 9.0 + SIDE_W + 10.0 + 12.0;
        let top = 9.0 + 36.0 + 10.0 + 12.0;
        let bottom = 9.0 + 12.0;
        Rectangle::new(
            Point::new(left, top),
            Size::new((size.width - left - right).max(1.0), (size.height - top - bottom).max(1.0)),
        )
    }

    /// Заставка при запуске, как у Фотошопа: картинка, название, версия, «что загружается»
    fn splash_view(&self) -> Element<'_, Message> {
        let elapsed = self.splash_start.map_or(0.0, |s| s.elapsed().as_secs_f32());
        let frac = (elapsed / SPLASH_SECS).min(1.0);
        const STEPS: [&str; 5] = [
            "Запуск ядра…",
            "Загрузка шрифтов…",
            "Подготовка кистей и кривых…",
            "Сборка режимов наложения…",
            "Почти готово…",
        ];
        let step = STEPS[((frac * STEPS.len() as f32) as usize).min(STEPS.len() - 1)];

        const BAR: f32 = 300.0;
        let progress = container(
            container(space().width(BAR * frac).height(2)).style(|_| container::background(ACCENT)),
        )
        .width(BAR)
        .height(2)
        .style(|_| container::background(BORDER));

        let info = column![
            text("SCAPT").size(48).color(TEXT),
            label("пиксельный редактор на Rust"),
            text(format!("версия {}", env!("CARGO_PKG_VERSION"))).size(10).color(MUTED),
            space().height(Length::Fill),
            text(step.to_uppercase()).size(10).color(MUTED),
            progress,
            label("клик — пропустить"),
        ]
        .spacing(10)
        .width(BAR)
        .height(300); // как картинка — иначе распорка Fill растянет карточку на всё окно

        let card = container(
            row![iced::widget::image(self.splash.clone()).width(256).height(300), info].spacing(28),
        )
        .padding(24)
        .style(panel_style(18.0));

        mouse_area(center(card)).on_press(Message::SkipSplash).into()
    }

    /// Настройки выделения
    fn select_panel(&self) -> Element<'_, Message> {
        let info = match self.ctx.selection {
            Some(r) => format!("{} × {} px  от ({}, {})", r.width(), r.height(), r.x0, r.y0),
            None => "нет выделения".to_string(),
        };
        let content = column![
            text("ВЫДЕЛЕНИЕ").size(12).color(TEXT),
            text(info.to_uppercase()).size(10).color(TEXT),
            row![
                text_button("Всё", false, Message::SelectAll),
                text_button("Снять", false, Message::Deselect),
            ]
            .spacing(6),
            text_button("Удалить содержимое", false, Message::DeleteSelection),
            label("Тянуть — прямоугольное выделение"),
            label("Клик — снять выделение"),
            label("Ctrl+A — всё, Ctrl+D / Esc — снять"),
            label("Delete — стереть внутри"),
            label("Кисть, ластик, заливка — только внутри"),
        ]
        .spacing(10)
        .padding(14);
        container(content).width(250).style(panel_style(16.0)).into()
    }

    /// Подсказки перемещения
    fn move_panel(&self) -> Element<'_, Message> {
        let what = if self.ctx.selection.is_some() { "выделенное" } else { "весь слой" };
        let content = column![
            text("ПЕРЕМЕЩЕНИЕ").size(12).color(TEXT),
            text(format!("двигается: {what}").to_uppercase()).size(10).color(TEXT),
            label("Тянуть — сдвинуть на активном слое"),
            label("Alt + тянуть — копия"),
            label("Без выделения двигается весь холст слоя"),
            label("Ctrl+Z — отменить сдвиг"),
        ]
        .spacing(10)
        .padding(14);
        container(content).width(250).style(panel_style(16.0)).into()
    }

    /// Блок выбора цвета: HEX, образец и ползунки R, G, B. Общий для кисти и заливки
    fn color_block(&self) -> Element<'_, Message> {
        let [r, g, b] = self.color;
        let swatch = container(space().width(22).height(22)).style(move |_| container::Style {
            background: Some(Background::Color(Color::from_rgb8(r, g, b))),
            border: Border { color: BORDER, width: 1.0, radius: 999.0.into() },
            ..container::Style::default()
        });
        let channel = |name: &'static str, i: usize, v: u8| {
            row![
                text(name).size(10).color(MUTED).width(12),
                slider(0..=255u8, v, move |v| Message::ColorChanged(i, v)),
                number_input(&self.rgb_text[i], move |s| Message::ChannelText(i, s)),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
        };
        column![
            row![
                label("Цвет"),
                space().width(Length::Fill),
                text_input("#RRGGBB", &self.hex_text).on_input(Message::HexText).size(10).padding([3, 6]).width(76),
                swatch,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            channel("R", 0, r),
            channel("G", 1, g),
            channel("B", 2, b),
        ]
        .spacing(10)
        .into()
    }

    /// Настройки заливки
    fn fill_panel(&self) -> Element<'_, Message> {
        let content = column![
            text("ЗАЛИВКА").size(12).color(TEXT),
            self.color_block(),
            slider_block(
                "Непрозрачность",
                text(format!("{}%", (self.opacity * 100.0).round())).size(10).color(TEXT),
                slider(0.0..=1.0, self.opacity, Message::OpacityChanged).step(0.01_f32),
            ),
            slider_block(
                "Допуск",
                text(self.tolerance).size(10).color(TEXT),
                slider(0..=255u8, self.tolerance, Message::ToleranceChanged),
            ),
            row![
                text_button("Смежные", self.contiguous, Message::Contiguous(true)),
                text_button("Все похожие", !self.contiguous, Message::Contiguous(false)),
            ]
            .spacing(6),
            label("Клик — залить область похожего цвета"),
            label("Цвет берётся с активного слоя"),
            label("Alt + клик — взять цвет пипеткой"),
        ]
        .spacing(10)
        .padding(14);

        container(content).width(250).style(panel_style(16.0)).into()
    }

    /// Настройки пипетки: какой цвет сейчас и откуда его брать
    fn picker_panel(&self) -> Element<'_, Message> {
        let [r, g, b] = self.color;
        let swatch = container(space().width(Length::Fill).height(48)).style(move |_| container::Style {
            background: Some(Background::Color(Color::from_rgb8(r, g, b))),
            border: Border { color: BORDER, width: 1.0, radius: 10.0.into() },
            ..container::Style::default()
        });
        let content = column![
            text("ПИПЕТКА").size(12).color(TEXT),
            swatch,
            row![
                label("Цвет кисти"),
                space().width(Length::Fill),
                text(to_hex(self.color)).size(10).color(TEXT),
            ],
            label("Брать цвет"),
            row![
                text_button("Все слои", self.pick_merged, Message::PickMerged(true)),
                text_button("Текущий слой", !self.pick_merged, Message::PickMerged(false)),
            ]
            .spacing(6),
            label("Клик или протянуть — взять цвет"),
            label("С кистью: Alt + клик"),
        ]
        .spacing(10)
        .padding(14);

        container(content).width(250).style(panel_style(16.0)).into()
    }

    /// Настройки лупы
    fn zoom_panel(&self) -> Element<'_, Message> {
        let z = self.current_zoom();
        let content = column![
            text("ЛУПА").size(12).color(TEXT),
            slider_block("Масштаб", text(format!("{}%", (z * 100.0).round())).size(10).color(TEXT), space()),
            row![
                text_button("−", false, Message::ZoomBy(1.0 / ZOOM_STEP)),
                text_button("+", false, Message::ZoomBy(ZOOM_STEP)),
            ]
            .spacing(6),
            row![
                text_button("Вписать", self.zoom.is_none(), Message::ZoomFit),
                text_button("100%", self.zoom == Some(1.0), Message::Zoom100),
            ]
            .spacing(6),
            checkbox(self.smooth_view)
                .label("СГЛАЖИВАТЬ ДО 400%")
                .text_size(10)
                .size(14)
                .on_toggle(Message::SmoothView),
            label("Клик — приблизить"),
            label("Alt + клик или ПКМ — отдалить"),
            label("Ctrl + колесо — масштаб"),
            label("Колесо / Shift + колесо — прокрутка"),
            label("Средняя кнопка — двигать холст"),
            label("Ctrl+0 — вписать, Ctrl+1 — 100%"),
        ]
        .spacing(10)
        .padding(14);

        container(content).width(250).style(panel_style(16.0)).into()
    }

    /// Строка под холстом: что произошло с файлом
    fn status_line(&self) -> Element<'_, Message> {
        match &self.status {
            Some((msg, is_error)) => text(msg.to_uppercase())
                .size(10)
                .color(if *is_error { DANGER } else { MUTED })
                .into(),
            None => text("").size(10).into(),
        }
    }

    /// Стартовый экран: «новый проект» с заготовками размеров или открыть файл
    fn start_view(&self) -> Element<'_, Message> {
        let mut nav = column![
            text("SCAPT").size(26).color(TEXT),
            label("пиксельный редактор"),
            space().height(24),
            text_button("Новый проект", true, Message::CloseMenus),
            text_button("Открыть…", false, Message::OpenProject),
        ]
        .spacing(8)
        .width(190);
        if self.has_project {
            nav = nav.push(text_button("Назад к рисунку", false, Message::BackToEditor));
        }

        // карточки заготовок, по три в ряд
        let card = |(name, w, h): (&str, u32, u32)| -> Element<'_, Message> {
            let active = self.new_w.trim() == w.to_string() && self.new_h.trim() == h.to_string();
            // маленький прямоугольник с пропорциями холста
            let k = 34.0 / w.max(h) as f32;
            let shape = container(space().width(w as f32 * k).height(h as f32 * k)).style(move |_| container::Style {
                border: Border { color: if active { BORDER_HI } else { BORDER }, width: 1.0, radius: 2.0.into() },
                ..container::Style::default()
            });
            button(
                column![
                    container(shape).center_x(Length::Fill).center_y(40),
                    text(name.to_uppercase()).size(10).color(TEXT),
                    text(format!("{w} × {h}")).size(9).color(MUTED),
                ]
                .spacing(4)
                .align_x(Alignment::Center),
            )
            .on_press(Message::NewPreset(w, h))
            .width(Length::Fill)
            .padding(10)
            .style(outline_button(active, 12.0))
            .into()
        };
        let presets = column(PRESETS.chunks(3).map(|chunk| {
            row(chunk.iter().map(|&p| card(p))).spacing(8).into()
        }))
        .spacing(8);

        let error: Element<'_, Message> = match &self.new_error {
            Some(e) => text(e.to_uppercase()).size(10).color(DANGER).into(),
            None => space().into(),
        };

        let size_field = |name: &str, value: &str, msg: fn(String) -> Message| {
            column![
                label(name),
                text_input("", value).on_input(msg).on_submit(Message::CreateProject).size(12).padding([6, 8]),
            ]
            .spacing(4)
        };

        let form = column![
            label("Имя"),
            text_input("Без имени", &self.new_name)
                .on_input(Message::NewName)
                .on_submit(Message::CreateProject)
                .size(12)
                .padding([6, 8]),
            row![
                size_field("Ширина, px", &self.new_w, Message::NewWidth),
                size_field("Высота, px", &self.new_h, Message::NewHeight),
            ]
            .spacing(8),
            label("Фон"),
            row![
                text_button("Белый", self.new_bg == NewBackground::White, Message::NewBg(NewBackground::White)),
                text_button(
                    "Прозрачный",
                    self.new_bg == NewBackground::Transparent,
                    Message::NewBg(NewBackground::Transparent),
                ),
            ]
            .spacing(6),
            space().height(Length::Fill),
            error,
            text_button("Создать", true, Message::CreateProject),
        ]
        .spacing(8)
        .width(230);

        let main = column![
            text("НОВЫЙ ПРОЕКТ").size(13).color(TEXT),
            row![container(presets).width(Length::Fill), form].spacing(20).height(330),
        ]
        .spacing(16);

        center(
            container(row![nav, container(space().width(1).height(Length::Fill)).style(|_| container::background(BORDER)), main].spacing(24))
                .padding(28)
                .width(980)
                .height(440)
                .style(panel_style(18.0)),
        )
        .into()
    }

    /// Настройки инструмента: цвет (только у кисти), размер, жёсткость, форма
    fn brush_panel(&self) -> Element<'_, Message> {
        let is_brush = self.tool == ToolKind::Brush;

        let shape_error: Element<'_, Message> = match &self.shape_error {
            Some(e) => text(e.clone()).size(10).color(DANGER).into(),
            None => space().into(),
        };

        let plot = container(
            cv::Canvas::new(CurvePlot { app: self, editable: false }).width(Length::Fill).height(80),
        )
        .style(panel_style(10.0));

        let preview = container(cv::Canvas::new(DabPreview { app: self }).width(84).height(84))
            .style(panel_style(10.0));

        let mut content = column![text(if is_brush { "КИСТЬ" } else { "ЛАСТИК" }).size(12).color(TEXT)]
            .spacing(10)
            .padding(14);
        if is_brush {
            content = content.push(self.color_block());
        }
        let content = content
            .push(slider_block(
                "Размер, px",
                number_input(&self.radius_text, Message::RadiusText),
                slider(1..=MAX_RADIUS, self.radius, Message::RadiusChanged),
            ))
            .push(slider_block(
                "Непрозрачность",
                text(format!("{}%", (self.opacity * 100.0).round())).size(10).color(TEXT),
                slider(0.0..=1.0, self.opacity, Message::OpacityChanged).step(0.01_f32),
            ))
            .push(slider_block(
                "Интервал",
                text(format!("{}%", (self.spacing * 100.0).round())).size(10).color(TEXT),
                slider(0.02..=1.0, self.spacing, Message::SpacingChanged).step(0.01_f32),
            ))
            .push(slider_block(
                "Сглаживание",
                text(format!("{}%", (self.smoothing * 100.0).round())).size(10).color(TEXT),
                slider(0.0..=0.9, self.smoothing, Message::SmoothingChanged).step(0.01_f32),
            ))
            .push(label("Жёсткость"))
            .push(plot)
            .push(
                row![
                    text_button("Кривая", false, Message::OpenCurve(true)),
                    checkbox(self.use_hardness)
                        .label(if self.shape.is_some() { "ПО КОНТУРУ" } else { "ВКЛ" })
                        .text_size(10)
                        .size(14)
                        .on_toggle(Message::UseHardness),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .push(label("Форма"))
            .push(
                text_input("путь к PNG/JPG", &self.shape_path)
                    .on_input(Message::ShapePathChanged)
                    .on_submit(Message::LoadShape)
                    .size(11)
                    .padding([5, 8]),
            )
            .push(
                row![
                    text_button("Загрузить", false, Message::LoadShape),
                    text_button("Круг", self.shape.is_none(), Message::ClearShape),
                ]
                .spacing(6),
            )
            .push(shape_error)
            .push(center(preview).height(Length::Shrink));

        container(scrollable(content))
            .width(250)
            .max_height(640)
            .style(panel_style(16.0))
            .into()
    }

    /// Панель слоёв: свойства активного слоя сверху, список слоёв снизу
    fn layers_panel(&self) -> Element<'_, Message> {
        let layer = &self.ctx.layers[self.ctx.active_layer];

        let props = column![
            label("Режим"),
            pick_list(&BlendMode::ALL[..], Some(layer.blend), Message::LayerBlend)
                .text_size(11)
                .padding([5, 8])
                .width(Length::Fill),
            slider_block(
                "Непрозрачность",
                text(format!("{}%", (layer.opacity * 100.0).round())).size(10).color(TEXT),
                slider(0.0..=1.0, layer.opacity, Message::LayerOpacity).step(0.01_f32),
            ),
            text_button("Очистить слой", false, Message::ClearLayer),
        ]
        .spacing(8)
        .padding([8, 6]);

        // сверху вниз, как в Фотошопе: последний слой в векторе — верхний
        let mut rows = column![].spacing(6).padding([1, 0]); // 1px: обводку не срезает прокрутка
        for (i, layer) in self.ctx.layers.iter().enumerate().rev() {
            let active = i == self.ctx.active_layer;
            let eye = icon_button(
                if layer.visible { IconKind::Eye } else { IconKind::EyeOff },
                false,
                20.0,
                Message::ToggleVisible(i, !layer.visible),
            );
            let content = row![
                eye,
                space().width(Length::Fill),
                text(layer.name.to_uppercase()).size(12).color(if active { TEXT } else { MUTED }),
                text(format!("{}%", (layer.opacity * 100.0).round())).size(8).color(MUTED),
            ]
            .spacing(6)
            .height(Length::Fill) // чтобы align_y центрировал по всей высоте строки
            .align_y(Alignment::Center);

            rows = rows.push(
                button(content)
                    .on_press(Message::SelectLayer(i))
                    .padding([0, 8])
                    .height(36)
                    .width(Length::Fill)
                    .style(outline_button(active, 10.0)),
            );
        }

        let panel = column![
            props,
            space().height(Length::Fill),
            container(scrollable(rows)).max_height(200),
        ]
        .padding(7);

        container(panel)
            .width(SIDE_W)
            .height(BAR_H)
            .style(panel_style(16.0))
            .into()
    }

    /// Полоса под слоями: добавить / удалить
    fn layers_bar(&self) -> Element<'_, Message> {
        container(
            row![
                icon_button(IconKind::Plus, false, 24.0, Message::AddLayer),
                icon_button(IconKind::Trash, false, 24.0, Message::DeleteLayer),
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .padding([0, 6])
        .center_y(36)
        .width(SIDE_W)
        .style(panel_style(18.0))
        .into()
    }

    fn curve_editor(&self) -> Element<'_, Message> {
        let mode_btn = |name: &str, mode| {
            text_button(name, self.curve_mode == mode, Message::SetCurveMode(mode)).width(110)
        };

        let source: Element<'_, Message> = match self.curve_mode {
            CurveMode::Formula => {
                let presets = row(["1 - t^2", "1 - t", "(1 - t^2)^2", "1 - t^4", "if(t < 0.7, 1, 0)"].map(|f| {
                    button(text(f).size(10))
                        .on_press(Message::FormulaChanged(f.to_string()))
                        .padding([4, 8])
                        .style(outline_button(self.formula == f, 10.0))
                        .into()
                }))
                .spacing(4);
                column![
                    label("f(t): t — от центра (0) до края (1)"),
                    text_input("например 1 - t^2", &self.formula)
                        .on_input(Message::FormulaChanged)
                        .size(12)
                        .padding([6, 8]),
                    presets,
                ]
                .spacing(8)
                .into()
            }
            CurveMode::Points => label("ЛКМ — добавить точку, тянуть — двигать, ПКМ — удалить").into(),
        };

        let error: Element<'_, Message> = match &self.formula_error {
            Some(e) => text(format!("Ошибка: {e}")).size(11).color(DANGER).into(),
            None => space().into(),
        };

        let window = column![
            row![
                text("КРИВАЯ ЖЁСТКОСТИ").size(13).color(TEXT),
                space().width(Length::Fill),
                text_button("Закрыть", false, Message::OpenCurve(false)).width(90),
            ]
            .align_y(Alignment::Center),
            row![mode_btn("Формула", CurveMode::Formula), mode_btn("Точки", CurveMode::Points)].spacing(6),
            source,
            row![
                container(
                    cv::Canvas::new(CurvePlot { app: self, editable: self.curve_mode == CurveMode::Points })
                        .width(420)
                        .height(300)
                )
                .style(panel_style(10.0)),
                container(cv::Canvas::new(DabPreview { app: self }).width(160).height(160))
                    .style(panel_style(10.0)),
            ]
            .spacing(12),
            error,
        ]
        .spacing(12);

        container(window).padding(18).style(panel_style(16.0)).width(640).into()
    }
}

/// Кисть-заглушка для Context::new (потом её сразу заменит rebuild_brush)
fn placeholder_brush() -> Brush {
    let curve = BoundaryFunc::from_formula("", "", 2, "1").unwrap();
    Brush::new("", "", "", [0, 0, 0, 255], 1, 1.0, curve, None).unwrap()
}

// ---------- график кривой (iced canvas) ----------

struct CurvePlot<'a> {
    app: &'a App,
    editable: bool,
}

#[derive(Default)]
struct PlotState {
    dragging: Option<usize>,
}

const PAD: f32 = 12.0;

impl CurvePlot<'_> {
    /// (t, alpha) -> точка на виджете
    fn to_screen(size: Size, t: f32, a: f32) -> Point {
        Point::new(PAD + t * (size.width - 2.0 * PAD), PAD + (1.0 - a) * (size.height - 2.0 * PAD))
    }

    /// точка на виджете -> (t, alpha), с обрезкой в [0, 1]
    fn from_screen(size: Size, p: Point) -> (f32, f32) {
        let t = (p.x - PAD) / (size.width - 2.0 * PAD);
        let a = 1.0 - (p.y - PAD) / (size.height - 2.0 * PAD);
        (t.clamp(0.0, 1.0), a.clamp(0.0, 1.0))
    }

    fn point_under(&self, size: Size, p: Point) -> Option<usize> {
        self.app.points.iter().position(|&(t, a)| Self::to_screen(size, t, a).distance(p) < 8.0)
    }
}

impl cv::Program<Message> for CurvePlot<'_> {
    type State = PlotState;

    fn update(
        &self,
        state: &mut PlotState,
        event: &cv::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<cv::Action<Message>> {
        if !self.editable {
            return None;
        }
        let size = bounds.size();
        match event {
            cv::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_in(bounds)?;
                match self.point_under(size, p) {
                    Some(i) => state.dragging = Some(i),
                    None => {
                        let (t, a) = Self::from_screen(size, p);
                        state.dragging = Some(self.app.points.len());
                        return Some(cv::Action::publish(Message::AddPoint(t, a)).and_capture());
                    }
                }
                Some(cv::Action::capture())
            }
            cv::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let i = state.dragging?;
                let p = cursor.position_in(bounds).or(cursor.position().map(|g| {
                    Point::new(g.x - bounds.x, g.y - bounds.y)
                }))?;
                let (t, a) = Self::from_screen(size, p);
                Some(cv::Action::publish(Message::MovePoint(i, t, a)).and_capture())
            }
            cv::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = None;
                None
            }
            cv::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                let p = cursor.position_in(bounds)?;
                let i = self.point_under(size, p)?;
                Some(cv::Action::publish(Message::RemovePoint(i)).and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &PlotState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);

        // сетка
        let grid = Stroke::default().with_width(1.0).with_color(GRID);
        for k in 0..=4 {
            let v = k as f32 / 4.0;
            frame.stroke(&Path::line(Self::to_screen(size, v, 0.0), Self::to_screen(size, v, 1.0)), grid);
            frame.stroke(&Path::line(Self::to_screen(size, 0.0, v), Self::to_screen(size, 1.0, v)), grid);
        }

        // сама кривая — по LUT, ровно то, что получит кисть
        let curve = Path::new(|b| {
            let n = 100;
            for k in 0..=n {
                let t = k as f32 / n as f32;
                let p = Self::to_screen(size, t, self.app.curve.sample(t));
                if k == 0 { b.move_to(p) } else { b.line_to(p) }
            }
        });
        frame.stroke(&curve, Stroke::default().with_width(2.0).with_color(ACCENT));

        // точки
        if self.editable {
            for &(t, a) in &self.app.points {
                frame.fill(&Path::circle(Self::to_screen(size, t, a), 5.0), TEXT);
            }
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &PlotState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if !self.editable {
            return mouse::Interaction::default();
        }
        match (state.dragging, cursor.position_in(bounds)) {
            (Some(_), _) => mouse::Interaction::Grabbing,
            (None, Some(p)) if self.point_under(bounds.size(), p).is_some() => mouse::Interaction::Grab,
            (None, Some(_)) => mouse::Interaction::Crosshair,
            _ => mouse::Interaction::default(),
        }
    }
}

// ---------- превью одного отпечатка кисти ----------

struct DabPreview<'a> {
    app: &'a App,
}

impl cv::Program<Message> for DabPreview<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        // светлая «бумага», скруглённая под рамку контейнера
        frame.fill(
            &Path::rounded_rectangle(Point::ORIGIN, size, 10.0.into()),
            color!(0xe9e6dc),
        );

        // у ластика «цвета» нет — показываем отпечаток серым
        let [r, g, b] = match self.app.tool {
            ToolKind::Brush => self.app.color,
            ToolKind::Fill => self.app.color,
            _ => [90, 88, 80],
        };

        // форма из картинки: рисуем маску по клеткам, альфа = shape * f(t) * opacity
        if let Some(shape) = &self.app.shape {
            let mask = shape.to_mask(30);
            let cell = (size.width.min(size.height) - 8.0) / mask.side as f32;
            let origin = Point::new(
                (size.width - cell * mask.side as f32) / 2.0,
                (size.height - cell * mask.side as f32) / 2.0,
            );
            for y in 0..mask.side {
                for x in 0..mask.side {
                    let i = y * mask.side + x;
                    let hard = if self.app.use_hardness { self.app.curve.sample(mask.t[i]) } else { 1.0 };
                    let alpha = mask.shape[i] * hard * self.app.opacity;
                    if alpha > 0.0 {
                        let p = Point::new(origin.x + x as f32 * cell, origin.y + y as f32 * cell);
                        // +0.5, чтобы между клетками не было щелей
                        frame.fill_rectangle(p, Size::new(cell + 0.5, cell + 0.5), Color::from_rgba8(r, g, b, alpha));
                    }
                }
            }
            return vec![frame.into_geometry()];
        }

        let center = Point::new(size.width / 2.0, size.height / 2.0);
        let radius = size.width.min(size.height) / 2.0 - 6.0;
        // кольца от края к центру: альфа каждого = f(t) * opacity
        let rings = 48;
        for k in (0..rings).rev() {
            let t = (k as f32 + 0.5) / rings as f32;
            let hard = if self.app.use_hardness { self.app.curve.sample(t) } else { 1.0 };
            let alpha = hard * self.app.opacity;
            let ring = Path::new(|p| {
                p.circle(center, radius * (k + 1) as f32 / rings as f32);
                if k > 0 {
                    p.circle(center, radius * k as f32 / rings as f32);
                }
            });
            frame.fill(&ring, cv::Fill {
                style: cv::Style::Solid(Color::from_rgba8(r, g, b, alpha)),
                rule: cv::fill::Rule::EvenOdd,
            });
        }
        vec![frame.into_geometry()]
    }
}

// ---------- холст на экране: плитки + обводка кисти ----------

struct CanvasView<'a> {
    app: &'a App,
}

/// Внутреннее состояние виджета: тащим ли холст средней кнопкой (последняя точка)
#[derive(Default)]
struct ViewState {
    panning: Option<Point>,
}

impl cv::Program<Message> for CanvasView<'_> {
    type State = ViewState;

    fn update(
        &self,
        state: &mut ViewState,
        event: &cv::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<cv::Action<Message>> {
        let app = self.app;
        let size = bounds.size();
        let (z, origin) = app.view_transform(size);
        // над панелью курсор «парит» (levitating) и позиции не даёт. Во время мазка или
        // перетаскивания она всё равно нужна — land() «приземляет» курсор обратно
        let busy = app.drawing || state.panning.is_some();
        let cur = if busy { cursor.land() } else { cursor };
        // курсор в координатах области (может быть и за её пределами)
        let local = cur.position().map(|p| Point::new(p.x - bounds.x, p.y - bounds.y));
        let inside = cursor.is_over(bounds);
        let publish = |m| Some(cv::Action::publish(m).and_capture());

        match event {
            cv::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let p = local?;
                if let Some(last) = state.panning {
                    state.panning = Some(p);
                    return publish(Message::Pan(p - last));
                }
                // во время мазка ведём его и за пределами области — мазок не обрывается
                if inside || app.drawing {
                    let c = (p - origin) * (1.0 / z);
                    return Some(cv::Action::publish(Message::MouseMoved(Point::new(c.x, c.y), inside)));
                }
                if app.hover {
                    return Some(cv::Action::publish(Message::HoverOut));
                }
                None
            }
            cv::Event::Mouse(mouse::Event::ButtonPressed(button)) if inside => {
                let p = local?;
                match (button, app.tool) {
                    (mouse::Button::Middle, _) => {
                        state.panning = Some(p);
                        Some(cv::Action::capture())
                    }
                    // лупа: клик — ближе, Alt+клик или правая кнопка — дальше
                    (mouse::Button::Left, ToolKind::Zoom) => {
                        let f = if app.modifiers.alt() { 1.0 / ZOOM_STEP } else { ZOOM_STEP };
                        publish(app.zoom_at(size, p, f))
                    }
                    (mouse::Button::Right, ToolKind::Zoom) => publish(app.zoom_at(size, p, 1.0 / ZOOM_STEP)),
                    (mouse::Button::Left, _) => publish(Message::MousePressed),
                    _ => None,
                }
            }
            // отпускание ловим где угодно в окне: canvas получает все события,
            // так что мазок больше не «залипает» (это был пункт 3 плана)
            cv::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)) => {
                state.panning = None;
                None
            }
            cv::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if app.drawing => {
                Some(cv::Action::publish(Message::MouseReleased))
            }
            cv::Event::Mouse(mouse::Event::WheelScrolled { delta }) if inside => {
                // строки (обычное колесо) или пиксели (тачпад) -> пиксели
                let (dx, dy) = match *delta {
                    mouse::ScrollDelta::Lines { x, y } => (x * 40.0, y * 40.0),
                    mouse::ScrollDelta::Pixels { x, y } => (x, y),
                };
                if app.modifiers.control() {
                    // один щелчок колеса = один шаг; знак — направление
                    let f = ZOOM_STEP.powf(dy.signum() * (dy.abs() / 40.0).clamp(0.25, 3.0));
                    publish(app.zoom_at(size, local?, f))
                } else if app.modifiers.shift() {
                    publish(Message::Pan(Vector::new(dy, dx)))
                } else {
                    publish(Message::Pan(Vector::new(dx, dy)))
                }
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &ViewState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let app = self.app;
        let (k, origin) = app.view_transform(bounds.size());
        let area = Rectangle::new(Point::ORIGIN, bounds.size());

        // рамка вокруг холста
        let (w, h) = (app.ctx.width as f32 * k, app.ctx.height as f32 * k);
        frame.stroke(
            &Path::rectangle(Point::new(origin.x - 0.5, origin.y - 0.5), Size::new(w + 1.0, h + 1.0)),
            Stroke::default().with_width(1.0).with_color(BORDER),
        );

        // Плитки. Текстура плитки — с рамкой TILE_PAD из соседей; рисуем её целиком, но обрезаем
        // по настоящей части плитки. Границы обрезки округлены до пикселей экрана и у соседей
        // совпадают — щелей нет, а на стыке сглаживание смешивает настоящие пиксели.
        // До 400% сглаживаем (Linear), дальше — чёткие пиксели (Nearest), как в Фотошопе
        let filter = if app.smooth_view && k < 4.0 { FilterMethod::Linear } else { FilterMethod::Nearest };
        let (tiles_x, tiles_y) = app.tiles_count();
        for ty in 0..tiles_y {
            for tx in 0..tiles_x {
                let (px, py) = (tx * TILE, ty * TILE);
                let (tw, th) = (TILE.min(app.ctx.width - px), TILE.min(app.ctx.height - py));
                // экранная граница по координате холста v
                let sx = |v: u32| (origin.x + v as f32 * k).round();
                let sy = |v: u32| (origin.y + v as f32 * k).round();
                let clip = Rectangle::new(
                    Point::new(sx(px), sy(py)),
                    Size::new(sx(px + tw) - sx(px), sy(py + th) - sy(py)),
                );
                // плитки за пределами видимой области не рисуем
                if clip.intersection(&area).is_none() {
                    continue;
                }
                let (srcx, srcy, sw, shh) = app.tile_source(tx, ty);
                let rect = Rectangle::new(
                    Point::new(origin.x + srcx as f32 * k, origin.y + srcy as f32 * k),
                    Size::new(sw as f32 * k, shh as f32 * k),
                );
                let handle = app.tiles[(ty * tiles_x + tx) as usize].clone();
                frame.with_clip(clip, |f| {
                    // координаты внутри with_clip — те же, что снаружи; clip только обрезает
                    f.draw_image(rect, cv::Image {
                        filter_method: filter,
                        snap: false,
                        ..cv::Image::new(handle)
                    });
                });
            }
        }

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(&self, state: &ViewState, bounds: Rectangle, cursor: mouse::Cursor) -> mouse::Interaction {
        if state.panning.is_some() {
            return mouse::Interaction::Grabbing;
        }
        if !cursor.is_over(bounds) {
            return mouse::Interaction::default();
        }
        match self.app.tool {
            ToolKind::Zoom if self.app.modifiers.alt() => mouse::Interaction::ZoomOut,
            ToolKind::Zoom => mouse::Interaction::ZoomIn,
            ToolKind::Move if self.app.moving.is_some() => mouse::Interaction::Grabbing,
            ToolKind::Move => mouse::Interaction::Grab,
            _ => mouse::Interaction::Crosshair,
        }
    }
}

// ---------- поверх холста: рамка выделения и обводка кисти ----------

/// Отдельный canvas поверх CanvasView (в Stack). Зачем отдельный: в одном слое отрисовки
/// iced рисует картинки ПОСЛЕ фигур, и линии оказались бы под плитками холста.
/// Событий не обрабатывает — они проходят к CanvasView под ним
struct Overlay<'a> {
    app: &'a App,
}

impl cv::Program<Message> for Overlay<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let app = self.app;
        let (k, origin) = app.view_transform(bounds.size());

        // рамка выделения: чёрная линия и белый пунктир поверх — видно на любом фоне
        if let Some(sel) = app.ctx.selection {
            let path = Path::rectangle(
                Point::new(origin.x + sel.x0 as f32 * k + 0.5, origin.y + sel.y0 as f32 * k + 0.5),
                Size::new(sel.width() as f32 * k - 1.0, sel.height() as f32 * k - 1.0),
            );
            frame.stroke(&path, Stroke::default().with_width(1.0).with_color(Color::BLACK));
            frame.stroke(&path, Stroke {
                line_dash: LineDash { segments: &[4.0, 4.0], offset: 0 },
                ..Stroke::default().with_width(1.0).with_color(Color::WHITE)
            });
        }

        let paints = matches!(app.tool, ToolKind::Brush | ToolKind::Eraser);
        if app.hover && paints && !app.modifiers.alt() {
            let p = app.mouse;
            // центр отпечатка — ровно под курсором (дробные координаты)
            let c = Point::new(origin.x + p.x * k, origin.y + p.y * k);
            let r = app.radius as f32 * k;
            let path = if app.shape.is_some() {
                // форма из картинки вписана в квадрат — обводим квадрат
                Path::rectangle(Point::new(c.x - r, c.y - r), Size::new(2.0 * r, 2.0 * r))
            } else {
                Path::circle(c, r)
            };
            // тёмная подложка + светлая линия: видно и на светлом, и на тёмном
            frame.stroke(&path, Stroke::default().with_width(3.0).with_color(Color::from_rgba8(0, 0, 0, 0.5)));
            frame.stroke(&path, Stroke::default().with_width(1.0).with_color(Color::from_rgba8(255, 255, 255, 0.9)));
        }

        vec![frame.into_geometry()]
    }
}

/// Горячая клавиша -> сообщение. Physical — физическая клавиша, поэтому Ctrl+Z работает
/// и на русской раскладке (там на этой клавише «я»)
fn shortcut(key: Physical, modifiers: Modifiers) -> Option<Message> {
    let Physical::Code(code) = key else { return None };
    // клавиши без Ctrl
    if !modifiers.command() {
        return match code {
            Code::Escape => Some(Message::Deselect),
            Code::Delete | Code::Backspace => Some(Message::DeleteSelection),
            _ => None,
        };
    }
    match code {
        Code::KeyA => Some(Message::SelectAll),
        Code::KeyD => Some(Message::Deselect),
        Code::KeyZ if modifiers.shift() => Some(Message::Redo),
        Code::KeyZ => Some(Message::Undo),
        Code::KeyY => Some(Message::Redo),
        Code::Digit0 | Code::Numpad0 => Some(Message::ZoomFit),
        Code::Digit1 | Code::Numpad1 => Some(Message::Zoom100),
        Code::Equal | Code::NumpadAdd => Some(Message::ZoomBy(ZOOM_STEP)),
        Code::Minus | Code::NumpadSubtract => Some(Message::ZoomBy(1.0 / ZOOM_STEP)),
        _ => None,
    }
}

// ---------- иконки, нарисованные линиями (в iced нет набора иконок) ----------

#[derive(Debug, Clone, Copy)]
enum IconKind {
    Brush,
    Select,
    Move,
    Fill,
    Eraser,
    Zoom,
    Picker,
    Plus,
    Trash,
    Eye,
    EyeOff,
}

struct Icon {
    kind: IconKind,
    color: Color,
}

impl<Message> cv::Program<Message> for Icon {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width.min(bounds.height);
        // координаты иконки задаём в долях 0..1, здесь переводим в пиксели
        let p = |x: f32, y: f32| Point::new(x * s, y * s);
        let stroke = Stroke {
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default().with_width(1.3).with_color(self.color)
        };

        let path = match self.kind {
            IconKind::Brush => Path::new(|b| {
                // карандаш по диагонали: корпус с заточкой и ободок
                b.move_to(p(0.66, 0.08));
                b.line_to(p(0.92, 0.34));
                b.line_to(p(0.36, 0.90));
                b.line_to(p(0.06, 0.94));
                b.line_to(p(0.10, 0.64));
                b.close();
                b.move_to(p(0.54, 0.20));
                b.line_to(p(0.80, 0.46));
            }),
            IconKind::Plus => Path::new(|b| {
                b.move_to(p(0.5, 0.12));
                b.line_to(p(0.5, 0.88));
                b.move_to(p(0.12, 0.5));
                b.line_to(p(0.88, 0.5));
            }),
            IconKind::Trash => Path::new(|b| {
                b.move_to(p(0.12, 0.24));
                b.line_to(p(0.88, 0.24));
                b.move_to(p(0.38, 0.24));
                b.line_to(p(0.38, 0.10));
                b.line_to(p(0.62, 0.10));
                b.line_to(p(0.62, 0.24));
                b.move_to(p(0.22, 0.24));
                b.line_to(p(0.28, 0.92));
                b.line_to(p(0.72, 0.92));
                b.line_to(p(0.78, 0.24));
                b.move_to(p(0.42, 0.42));
                b.line_to(p(0.42, 0.74));
                b.move_to(p(0.58, 0.42));
                b.line_to(p(0.58, 0.74));
            }),
            IconKind::Eye | IconKind::EyeOff => Path::new(|b| {
                b.move_to(p(0.04, 0.5));
                b.quadratic_curve_to(p(0.5, 0.06), p(0.96, 0.5));
                b.quadratic_curve_to(p(0.5, 0.94), p(0.04, 0.5));
                b.circle(p(0.5, 0.5), 0.14 * s);
                if let IconKind::EyeOff = self.kind {
                    b.move_to(p(0.12, 0.88));
                    b.line_to(p(0.88, 0.12));
                }
            }),
            IconKind::Select => Path::new(|b| {
                // пунктирный квадрат: по два штриха на сторону
                let segs = [
                    ((0.10, 0.10), (0.36, 0.10)), ((0.64, 0.10), (0.90, 0.10)),
                    ((0.90, 0.10), (0.90, 0.36)), ((0.90, 0.64), (0.90, 0.90)),
                    ((0.90, 0.90), (0.64, 0.90)), ((0.36, 0.90), (0.10, 0.90)),
                    ((0.10, 0.90), (0.10, 0.64)), ((0.10, 0.36), (0.10, 0.10)),
                ];
                for ((ax, ay), (bx, by)) in segs {
                    b.move_to(p(ax, ay));
                    b.line_to(p(bx, by));
                }
            }),
            IconKind::Move => Path::new(|b| {
                // крест со стрелками на все четыре стороны
                b.move_to(p(0.5, 0.06));
                b.line_to(p(0.5, 0.94));
                b.move_to(p(0.06, 0.5));
                b.line_to(p(0.94, 0.5));
                for (a, tip, c) in [
                    ((0.36, 0.20), (0.5, 0.06), (0.64, 0.20)),
                    ((0.36, 0.80), (0.5, 0.94), (0.64, 0.80)),
                    ((0.20, 0.36), (0.06, 0.5), (0.20, 0.64)),
                    ((0.80, 0.36), (0.94, 0.5), (0.80, 0.64)),
                ] {
                    b.move_to(p(a.0, a.1));
                    b.line_to(p(tip.0, tip.1));
                    b.line_to(p(c.0, c.1));
                }
            }),
            IconKind::Fill => Path::new(|b| {
                // ведро, наклонённое ромбом, и капля справа
                b.move_to(p(0.10, 0.48));
                b.line_to(p(0.44, 0.14));
                b.line_to(p(0.80, 0.50));
                b.line_to(p(0.46, 0.84));
                b.close();
                b.move_to(p(0.10, 0.48));
                b.line_to(p(0.80, 0.50));
                b.move_to(p(0.88, 0.62));
                b.line_to(p(0.88, 0.80));
                b.circle(p(0.88, 0.86), 0.05 * s);
            }),
            IconKind::Picker => Path::new(|b| {
                // пипетка: груша сверху справа, стеклянная трубка, кончик внизу слева
                b.circle(p(0.76, 0.24), 0.14 * s);
                b.move_to(p(0.58, 0.30));
                b.line_to(p(0.70, 0.42));
                b.move_to(p(0.64, 0.36));
                b.line_to(p(0.24, 0.76));
                b.line_to(p(0.10, 0.90));
                b.move_to(p(0.64, 0.36));
                b.line_to(p(0.30, 0.70));
            }),
            IconKind::Zoom => Path::new(|b| {
                // стекло и ручка
                b.circle(p(0.42, 0.42), 0.30 * s);
                b.move_to(p(0.64, 0.64));
                b.line_to(p(0.92, 0.92));
            }),
            IconKind::Eraser => Path::new(|b| {
                // резинка по диагонали: корпус, граница рабочей части, линия «стола»
                b.move_to(p(0.60, 0.08));
                b.line_to(p(0.92, 0.40));
                b.line_to(p(0.46, 0.86));
                b.line_to(p(0.30, 0.86));
                b.line_to(p(0.10, 0.66));
                b.close();
                b.move_to(p(0.30, 0.38));
                b.line_to(p(0.62, 0.70));
                b.move_to(p(0.46, 0.94));
                b.line_to(p(0.92, 0.94));
            }),
        };
        frame.stroke(&path, stroke);
        vec![frame.into_geometry()]
    }
}
