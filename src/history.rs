//! История действий для отмены (Ctrl+Z) и повтора (Ctrl+Y).
//!
//! Два вида записей:
//! - мазок — храним только задетые пиксели одного слоя: какими были до и стали после.
//!   Это дёшево: мазок по 1000 пикселям = 1000 записей, а не копия всего слоя;
//! - операция со слоями (добавить, удалить, очистить, импорт) — храним снимок всех слоёв
//!   до и после. Проще и надёжнее, но дороже по памяти, поэтому история ограничена.

use crate::canvas::{Layer, Pixel};
use crate::context::Context;
use crate::tools::Stroke;

/// Сколько шагов назад помним
const LIMIT: usize = 50;

pub enum Action {
    /// Мазок: (координаты, пиксель до, пиксель после) на слое layer
    Pixels {
        layer: usize,
        changes: Vec<((i32, i32), Pixel, Pixel)>,
    },
    /// Изменение набора слоёв: (слои, активный слой) до и после
    Layers {
        before: (Vec<Layer>, usize),
        after: (Vec<Layer>, usize),
    },
}

/// Что перерисовать после отмены/повтора
pub enum Dirty {
    /// Прямоугольник [x0, x1) × [y0, y1)
    Rect(i32, i32, i32, i32),
    /// Весь холст
    All,
}

#[derive(Default)]
pub struct History {
    undo: Vec<Action>,
    redo: Vec<Action>,
}

impl History {
    /// Записать новое действие. Ветка «вперёд» после этого теряет смысл и очищается —
    /// как в любом редакторе: отменил, нарисовал новое — повторить старое уже нельзя
    pub fn push(&mut self, action: Action) {
        self.undo.push(action);
        self.redo.clear();
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    /// Записать законченный мазок: «до» берём из Stroke.before, «после» — из слоя сейчас
    pub fn push_stroke(&mut self, ctx: &Context, layer: usize, stroke: Stroke) {
        let Some(l) = ctx.layers.get(layer) else { return };
        let changes: Vec<_> = stroke
            .before
            .into_iter()
            .map(|(pos, before)| (pos, before, l.get_pixel(pos.0, pos.1)))
            .filter(|&(_, before, after)| before != after) // пиксели, которые на деле не изменились, не храним
            .collect();
        if !changes.is_empty() {
            self.push(Action::Pixels { layer, changes });
        }
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// Отменить последнее действие. None — отменять нечего
    pub fn undo(&mut self, ctx: &mut Context) -> Option<Dirty> {
        let action = self.undo.pop()?;
        let dirty = apply(ctx, &action, false);
        self.redo.push(action);
        Some(dirty)
    }

    /// Повторить отменённое. None — повторять нечего
    pub fn redo(&mut self, ctx: &mut Context) -> Option<Dirty> {
        let action = self.redo.pop()?;
        let dirty = apply(ctx, &action, true);
        self.undo.push(action);
        Some(dirty)
    }
}

/// Применить действие: forward = true — «после» (повтор), false — «до» (отмена)
fn apply(ctx: &mut Context, action: &Action, forward: bool) -> Dirty {
    match action {
        Action::Pixels { layer, changes } => {
            let Some(l) = ctx.layers.get_mut(*layer) else { return Dirty::All };
            let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            for &((x, y), before, after) in changes {
                l.set_pixel(x, y, if forward { after } else { before });
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
            Dirty::Rect(x0, y0, x1, y1)
        }
        Action::Layers { before, after } => {
            let (layers, active) = if forward { after } else { before };
            // clone: снимок остаётся в истории, чтобы его можно было применить снова
            ctx.layers = layers.clone();
            ctx.active_layer = *active;
            Dirty::All
        }
    }
}
