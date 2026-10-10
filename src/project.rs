//! Сохранение и загрузка: свой формат проекта .scapt и экспорт/импорт PNG.
//!
//! Формат .scapt (все числа little-endian):
//!
//! ```text
//! "SCAPT"           5 байт — «магия», чтобы отличить наш файл от чужого
//! version   u32     сейчас 1
//! width     u32     размер холста
//! height    u32
//! block     u32     сторона блока (64)
//! active    u32     номер активного слоя
//! layers    u32     сколько слоёв, дальше каждый слой:
//!   name_len  u32, name — UTF-8
//!   blend     u8    номер в BlendMode::ALL
//!   opacity   f32
//!   visible   u8    0 / 1
//!   blocks    u32   сколько блоков, дальше каждый блок:
//!     bx, by  i32   координаты блока
//!     pixels  block * block * 4 байт, RGBA premultiplied
//! ```
//!
//! Пустые (полностью прозрачные) блоки не пишем — файл занимает место только там, где рисовали.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};

use crate::canvas::{BlendMode, GridBlock, Layer, Pixel, TRANSPARENT};
use crate::context::Context;

const MAGIC: &[u8; 5] = b"SCAPT";
const VERSION: u32 = 1;

// ---------- проект ----------

/// Сохранить все слои проекта в файл .scapt
pub fn save_project(ctx: &Context, path: &str) -> Result<(), String> {
    write_project(ctx, path).map_err(|e| format!("не удалось сохранить {path}: {e}"))
}

fn write_project(ctx: &Context, path: &str) -> io::Result<()> {
    // BufWriter копит мелкие записи и пишет на диск большими кусками — как buffering в open() в Python
    let mut w = BufWriter::new(File::create(path)?);

    w.write_all(MAGIC)?;
    write_u32(&mut w, VERSION)?;
    write_u32(&mut w, ctx.width)?;
    write_u32(&mut w, ctx.height)?;
    write_u32(&mut w, ctx.block_size as u32)?;
    write_u32(&mut w, ctx.active_layer as u32)?;
    write_u32(&mut w, ctx.layers.len() as u32)?;

    for layer in &ctx.layers {
        let name = layer.name.as_bytes();
        write_u32(&mut w, name.len() as u32)?;
        w.write_all(name)?;
        w.write_all(&[layer.blend.index()])?;
        w.write_all(&layer.opacity.to_le_bytes())?;
        w.write_all(&[layer.visible as u8])?;

        let blocks: Vec<&GridBlock> = layer.blocks().filter(|b| !b.is_empty()).collect();
        write_u32(&mut w, blocks.len() as u32)?;
        for block in blocks {
            w.write_all(&block.position.0.to_le_bytes())?;
            w.write_all(&block.position.1.to_le_bytes())?;
            // [[u8; 4]] -> плоские байты: as_flattened превращает срез массивов в один срез
            w.write_all(block.pixels().as_flattened())?;
        }
    }
    w.flush()
}

/// Загрузить проект из файла .scapt в ctx (инструмент и курсор остаются прежними).
/// Если файл битый — ctx не меняется.
pub fn load_project(ctx: &mut Context, path: &str) -> Result<(), String> {
    let loaded = read_project(path).map_err(|e| match e.kind() {
        // read_exact упёрся в конец файла раньше, чем ожидалось
        io::ErrorKind::UnexpectedEof => format!("не удалось открыть {path}: файл обрезан или повреждён"),
        _ => format!("не удалось открыть {path}: {e}"),
    })?;
    ctx.width = loaded.width;
    ctx.height = loaded.height;
    ctx.block_size = loaded.block_size;
    ctx.layers = loaded.layers;
    ctx.active_layer = loaded.active.min(ctx.layers.len() - 1);
    Ok(())
}

/// Всё прочитанное из файла, пока не положено в Context
struct Loaded {
    width: u32,
    height: u32,
    block_size: usize,
    active: usize,
    layers: Vec<Layer>,
}

fn read_project(path: &str) -> io::Result<Loaded> {
    let mut r = BufReader::new(File::open(path)?);

    let mut magic = [0u8; 5];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(bad("это не файл Scapt"));
    }
    let version = read_u32(&mut r)?;
    if version != VERSION {
        return Err(bad(&format!("неизвестная версия формата: {version}")));
    }

    let width = read_u32(&mut r)?;
    let height = read_u32(&mut r)?;
    let block_size = read_u32(&mut r)? as usize;
    // защита от мусора в файле: иначе vec! ниже попробует выделить гигабайты
    if !(1..=1024).contains(&block_size) {
        return Err(bad(&format!("странный размер блока: {block_size}")));
    }
    let active = read_u32(&mut r)? as usize;
    let layer_count = read_u32(&mut r)?;
    if layer_count == 0 {
        return Err(bad("в проекте нет слоёв"));
    }

    let mut layers = Vec::new();
    for _ in 0..layer_count {
        let name_len = read_u32(&mut r)? as usize;
        if name_len > 4096 {
            return Err(bad("слишком длинное имя слоя"));
        }
        let mut name = vec![0u8; name_len];
        r.read_exact(&mut name)?;
        let name = String::from_utf8(name).map_err(|_| bad("имя слоя — не UTF-8"))?;

        let mut layer = Layer::new(&name, block_size);
        layer.blend = BlendMode::from_index(read_u8(&mut r)?).ok_or_else(|| bad("неизвестный режим наложения"))?;
        layer.opacity = read_f32(&mut r)?.clamp(0.0, 1.0);
        layer.visible = read_u8(&mut r)? != 0;

        let block_count = read_u32(&mut r)?;
        for _ in 0..block_count {
            let bx = read_i32(&mut r)?;
            let by = read_i32(&mut r)?;
            let mut bytes = vec![0u8; block_size * block_size * 4];
            r.read_exact(&mut bytes)?;
            // плоские байты -> [u8; 4] по четыре
            let pixels: Vec<Pixel> = bytes
                .chunks_exact(4)
                .map(|c| [c[0], c[1], c[2], c[3]])
                .collect();
            layer.insert_block(GridBlock::from_pixels((bx, by), pixels));
        }
        layers.push(layer);
    }

    Ok(Loaded { width, height, block_size, active, layers })
}

// ---------- PNG ----------

/// Склеить все видимые слои в картинку width × height и сохранить в PNG
pub fn export_png(ctx: &Context, path: &str) -> Result<(), String> {
    let (w, h) = (ctx.width, ctx.height);
    let bs = ctx.block_size as i32;
    let mut img = image::RgbaImage::new(w, h);
    let mut buf = Vec::new();

    // собираем по блокам, как render_rect в main.rs
    for by in 0..(h as i32 + bs - 1) / bs {
        for bx in 0..(w as i32 + bs - 1) / bs {
            ctx.composite_block((bx, by), &mut buf);
            for ly in 0..bs {
                for lx in 0..bs {
                    let (x, y) = (bx * bs + lx, by * bs + ly);
                    if x < w as i32 && y < h as i32 {
                        let p = unpremultiply(buf[(ly * bs + lx) as usize]);
                        img.put_pixel(x as u32, y as u32, image::Rgba(p));
                    }
                }
            }
        }
    }
    img.save(path).map_err(|e| format!("не удалось сохранить {path}: {e}"))
}

/// Открыть картинку и добавить её новым слоем в левый верхний угол холста
pub fn import_png(ctx: &mut Context, path: &str) -> Result<(), String> {
    let img = image::open(path)
        .map_err(|e| format!("не удалось открыть {path}: {e}"))?
        .to_rgba8();

    let name = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Картинка".to_string());
    ctx.add_layer(&name);
    let layer = ctx.layers.last_mut().unwrap();

    for (x, y, p) in img.enumerate_pixels() {
        let px = premultiply(p.0);
        if px[3] > 0 {
            layer.set_pixel(x as i32, y as i32, px);
        }
    }
    Ok(())
}

/// В слое цвета умножены на альфу; в PNG — нет. Делим обратно.
pub fn unpremultiply([r, g, b, a]: Pixel) -> Pixel {
    if a == 0 {
        return TRANSPARENT;
    }
    let k = 255.0 / a as f32;
    let f = |c: u8| (c as f32 * k).round().min(255.0) as u8;
    [f(r), f(g), f(b), a]
}

/// Цвет из PNG -> как хранится в слое: умножаем на альфу
fn premultiply([r, g, b, a]: Pixel) -> Pixel {
    let k = a as f32 / 255.0;
    let f = |c: u8| (c as f32 * k).round() as u8;
    [f(r), f(g), f(b), a]
}

// ---------- чтение и запись чисел ----------

fn write_u32(w: &mut impl Write, v: u32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_i32(r: &mut impl Read) -> io::Result<i32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}

fn read_f32(r: &mut impl Read) -> io::Result<f32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(f32::from_le_bytes(b))
}

/// Ошибка «файл испорчен» с понятным текстом
fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}
