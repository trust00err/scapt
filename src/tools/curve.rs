use evalexpr::*;

/// Способ задать кривую жёсткости
pub enum CurveSource {
    /// Точки (t, alpha), отсортированные по t
    Points(Vec<(f32, f32)>),
    /// Формула от t, например "1 - t^2"
    Formula(String),
}

/// Кривая жёсткости кисти: t ∈ [0,1] (центр → край) → alpha ∈ [0,1]
pub struct BoundaryFunc {
    name: String,
    description: String,
    source: CurveSource,
    lut: Vec<f32>,
    lut_size: usize,
}

impl BoundaryFunc {
    pub fn from_points(name: &str, description: &str, lut_size: usize, mut points: Vec<(f32, f32)>) -> Self {
        points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut f = Self {
            name: name.to_string(),
            description: description.to_string(),
            source: CurveSource::Points(points),
            lut: Vec::new(),
            lut_size,
        };
        f.rebuild_lut();
        f
    }

    /// Кривая из формулы. Возвращает ошибку, если формула некорректна.
    pub fn from_formula(name: &str, description: &str, lut_size: usize, formula: &str) -> Result<Self, String> {
        eval_formula(formula, 0.5)?;

        let mut f = Self {
            name: name.to_string(),
            description: description.to_string(),
            source: CurveSource::Formula(formula.to_string()),
            lut: Vec::new(),
            lut_size,
        };
        f.rebuild_lut();
        Ok(f)
    }

    fn rebuild_lut(&mut self) {
        let n = self.lut_size.max(2);
        self.lut = (0..n)
            .map(|i| {
                let t = i as f32 / (n - 1) as f32;
                self.compute(t).clamp(0.0, 1.0)
            })
            .collect();
    }

    /// Честное вычисление f(t)
    fn compute(&self, t: f32) -> f32 {
        match &self.source {
            CurveSource::Points(pts) => interpolate(pts, t),
            CurveSource::Formula(text) => eval_formula(text, t).unwrap_or(0.0),
        }
    }

    /// Быстрое значение из таблицы - для каждого пикселя
    pub fn sample(&self, t: f32) -> f32 {
        let n = self.lut.len();
        let i = (t.clamp(0.0, 1.0) * (n - 1) as f32) as usize;
        self.lut[i]
    }
}

/// Вычислить формулу при заданном t
fn eval_formula(formula: &str, t: f32) -> Result<f32, String> {
    let mut ctx = HashMapContext::<DefaultNumericTypes>::new();
    ctx.set_value("t".into(), Value::Float(t as f64))
        .map_err(|e| e.to_string())?;
    let v = eval_number_with_context(formula, &ctx).map_err(|e| e.to_string())?;
    Ok(v as f32)
}

/// Линейная интерполяция между соседними точками
fn interpolate(pts: &[(f32, f32)], t: f32) -> f32 {
    if pts.is_empty() { return 1.0; }
    if t <= pts[0].0 { return pts[0].1; }
    for w in pts.windows(2) {
        let (x0, y0) = w[0];
        let (x1, y1) = w[1];
        if t <= x1 {
            let k = (t - x0) / (x1 - x0);
            return y0 + (y1 - y0) * k;
        }
    }
    pts[pts.len() - 1].1
}