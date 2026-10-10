/// Форма кисти, загруженная из картинки, в её родном разрешении.
/// Под конкретный радиус из неё строится ShapeMask (см. to_mask).
pub struct BrushShape {
    width: usize,
    height: usize,
    /// Сколько краски кладёт кисть: 0..1, индекс y * width + x
    data: Vec<f32>,
    /// «Координата жёсткости» t для каждого пикселя (вариант В):
    /// 0 — самая глубокая точка формы, 1 — её контур и всё снаружи.
    /// Для круга совпадает с обычным t = расстояние / радиус.
    t: Vec<f32>,
}

/// Форма, ужатая под радиус: квадрат side × side, side = 2 * radius + 1
pub struct ShapeMask {
    pub side: usize,
    pub shape: Vec<f32>,
    pub t: Vec<f32>,
}

impl BrushShape {
    /// Загрузить форму из картинки.
    /// Есть прозрачность — берём альфу, нет — чёрный = краска, белый = пусто.
    pub fn from_image(path: &str) -> Result<Self, String> {
        let img = image::open(path).map_err(|e| format!("{path}: {e}"))?;
        let has_alpha = img.color().has_alpha();
        let la = img.to_luma_alpha8();
        let (width, height) = (la.width() as usize, la.height() as usize);

        let data: Vec<f32> = la
            .pixels()
            .map(|p| {
                let [l, a] = p.0;
                if has_alpha { a as f32 / 255.0 } else { 1.0 - l as f32 / 255.0 }
            })
            .collect();

        if data.iter().all(|&v| v < 0.5) {
            return Err("в картинке нет формы: всё пусто (белое или прозрачное)".to_string());
        }

        let t = hardness_field(&data, width, height);
        Ok(Self { width, height, data, t })
    }

    /// Ужать (или растянуть) форму под кисть радиуса radius.
    /// Картинка вписывается в квадрат по длинной стороне и центрируется.
    pub fn to_mask(&self, radius: i32) -> ShapeMask {
        let side = (2 * radius.max(1) + 1) as usize;
        ShapeMask {
            side,
            shape: self.resample(&self.data, side, 0.0),
            t: self.resample(&self.t, side, 1.0),
        }
    }

    /// Усреднение по области (box filter): каждый пиксель результата — среднее
    /// всех пикселей src, которые на него приходятся. outside — значение за краем картинки.
    fn resample(&self, src: &[f32], side: usize, outside: f32) -> Vec<f32> {
        let (w, h, s) = (self.width as f32, self.height as f32, side as f32);
        let scale = w.max(h) / s; // сколько пикселей картинки на один пиксель маски
        let off_x = (s * scale - w) / 2.0; // поля, чтобы картинка была по центру
        let off_y = (s * scale - h) / 2.0;

        // диапазон пикселей картинки [lo, hi), попадающий в пиксель маски i;
        // хотя бы один пиксель — на случай, когда картинка меньше кисти
        let span = |i: usize, off: f32, len: usize| {
            let a = i as f32 * scale - off;
            let b = a + scale;
            let lo = a.floor() as i64;
            let hi = (b.ceil() as i64).max(lo + 1);
            (lo.max(0) as usize, hi.min(len as i64).max(0) as usize)
        };

        let mut out = Vec::with_capacity(side * side);
        for my in 0..side {
            let (y0, y1) = span(my, off_y, self.height);
            for mx in 0..side {
                let (x0, x1) = span(mx, off_x, self.width);
                if x0 >= x1 || y0 >= y1 {
                    out.push(outside); // поле вокруг картинки
                    continue;
                }
                let mut sum = 0.0;
                for y in y0..y1 {
                    sum += src[y * self.width + x0..y * self.width + x1].iter().sum::<f32>();
                }
                out.push(sum / ((x1 - x0) * (y1 - y0)) as f32);
            }
        }
        out
    }
}

// ---------- вариант В: жёсткость по контуру формы ----------

/// Для каждого пикселя: t = 1 - (расстояние до контура / максимальное расстояние).
/// Внутри формы — пиксели со значением >= 0.5, за краем картинки — снаружи.
fn hardness_field(data: &[f32], w: usize, h: usize) -> Vec<f32> {
    // рамка в 1 пиксель «снаружи», чтобы край картинки тоже считался контуром
    let (pw, ph) = (w + 2, h + 2);
    // «бесконечность»: больше любого возможного квадрата расстояния.
    // f64::INFINITY не годится — в формуле есть INF - INF, а это NaN
    let far = ((pw + ph) * (pw + ph)) as f64;
    let mut grid = vec![0.0f64; pw * ph];
    for y in 0..h {
        for x in 0..w {
            if data[y * w + x] >= 0.5 {
                grid[(y + 1) * pw + (x + 1)] = far;
            }
        }
    }

    distance_transform(&mut grid, pw, ph);

    let mut dist = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            dist.push(grid[(y + 1) * pw + (x + 1)].sqrt() as f32);
        }
    }
    let max = dist.iter().cloned().fold(0.0f32, f32::max);
    if max == 0.0 {
        return vec![1.0; w * h];
    }
    dist.iter().map(|&d| if d > 0.0 { 1.0 - d / max } else { 1.0 }).collect()
}

/// Точный евклидов distance transform (Felzenszwalb & Huttenlocher, 2012).
/// На входе: 0 — снаружи формы, «бесконечность» — внутри. На выходе: КВАДРАТ
/// расстояния до ближайшего пикселя снаружи. Два прохода 1D: столбцы, потом строки.
/// Считаем в f64: в f32 большие числа теряют точность, и формула ломается.
fn distance_transform(grid: &mut [f64], w: usize, h: usize) {
    let n = w.max(h);
    let (mut f, mut d) = (vec![0.0; n], vec![0.0; n]);
    let (mut v, mut z) = (vec![0usize; n], vec![0.0f64; n + 1]);

    for x in 0..w {
        for y in 0..h {
            f[y] = grid[y * w + x];
        }
        dt_1d(&f[..h], &mut d[..h], &mut v, &mut z);
        for y in 0..h {
            grid[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&grid[y * w..(y + 1) * w]);
        dt_1d(&f[..w], &mut d[..w], &mut v, &mut z);
        grid[y * w..(y + 1) * w].copy_from_slice(&d[..w]);
    }
}

/// Одномерный проход: d[q] = min по p от ((q - p)² + f[p]) — нижняя огибающая парабол.
/// v — вершины парабол в огибающей, z — границы, где одна парабола сменяет другую.
fn dt_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let sq = |i: usize| (i * i) as f64;
    let mut k = 0;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        // пересечение параболы q с последней в огибающей; если оно левее границы —
        // последняя парабола нигде не минимальна, выкидываем её
        let mut s = ((f[q] + sq(q)) - (f[v[k]] + sq(v[k]))) / (2.0 * (q - v[k]) as f64);
        while s <= z[k] {
            k -= 1;
            s = ((f[q] + sq(q)) - (f[v[k]] + sq(v[k]))) / (2.0 * (q - v[k]) as f64);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        d[q] = (q as f64 - v[k] as f64).powi(2) + f[v[k]];
    }
}
