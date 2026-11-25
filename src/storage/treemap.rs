use egui::{pos2, vec2, Rect};

/// Treemap "squarified": recebe (id, valor) em ordem decrescente e devolve (id, retângulo).
pub fn layout(items: &[(usize, f64)], rect: Rect) -> Vec<(usize, Rect)> {
    let total: f64 = items.iter().map(|i| i.1).sum();
    if items.is_empty() || total <= 0.0 || rect.width() <= 1.0 || rect.height() <= 1.0 {
        return vec![];
    }
    let scale = (rect.width() as f64 * rect.height() as f64) / total;
    let vals: Vec<(usize, f64)> = items.iter().map(|&(i, v)| (i, v * scale)).collect();

    let mut out = Vec::with_capacity(vals.len());
    let mut r = rect;
    let mut row: Vec<(usize, f64)> = Vec::new();
    let mut i = 0;
    while i < vals.len() {
        if r.width() <= 0.5 || r.height() <= 0.5 {
            break;
        }
        let side = r.width().min(r.height()) as f64;
        if row.is_empty() {
            row.push(vals[i]);
            i += 1;
            continue;
        }
        let mut cand = row.clone();
        cand.push(vals[i]);
        if worst(&cand, side) <= worst(&row, side) {
            row = cand;
            i += 1;
        } else {
            r = place(&row, r, &mut out);
            row.clear();
        }
    }
    if !row.is_empty() && r.width() > 0.5 && r.height() > 0.5 {
        place(&row, r, &mut out);
    }
    out
}

fn worst(row: &[(usize, f64)], side: f64) -> f64 {
    let s: f64 = row.iter().map(|x| x.1).sum();
    let max = row.iter().map(|x| x.1).fold(f64::MIN, f64::max);
    let min = row.iter().map(|x| x.1).fold(f64::MAX, f64::min);
    ((side * side * max) / (s * s)).max((s * s) / (side * side * min))
}

fn place(row: &[(usize, f64)], r: Rect, out: &mut Vec<(usize, Rect)>) -> Rect {
    let s: f64 = row.iter().map(|x| x.1).sum();
    if r.width() >= r.height() {
        // linha vira uma coluna à esquerda
        let w = (s / r.height() as f64) as f32;
        let mut y = r.top();
        for &(i, v) in row {
            let h = (v / s * r.height() as f64) as f32;
            out.push((i, Rect::from_min_size(pos2(r.left(), y), vec2(w, h))));
            y += h;
        }
        Rect::from_min_max(pos2(r.left() + w, r.top()), r.max)
    } else {
        // linha vira uma faixa no topo
        let h = (s / r.width() as f64) as f32;
        let mut x = r.left();
        for &(i, v) in row {
            let w = (v / s * r.width() as f64) as f32;
            out.push((i, Rect::from_min_size(pos2(x, r.top()), vec2(w, h))));
            x += w;
        }
        Rect::from_min_max(pos2(r.left(), r.top() + h), r.max)
    }
}
