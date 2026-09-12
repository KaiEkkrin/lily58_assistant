//! Physical key layout from the Vial definition (KLE format, as parsed by vial-gui's kle_serial.py).

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct KeyGeom {
    pub row: u8,
    pub col: u8,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Degrees, clockwise, about (rx, ry).
    pub angle: f32,
    pub rx: f32,
    pub ry: f32,
}

impl KeyGeom {
    /// Corners after rotation, clockwise from top-left, in key units.
    pub fn corners(&self) -> [(f32, f32); 4] {
        let (s, c) = self.angle.to_radians().sin_cos();
        let rot = |px: f32, py: f32| {
            let (dx, dy) = (px - self.rx, py - self.ry);
            (self.rx + dx * c - dy * s, self.ry + dx * s + dy * c)
        };
        [
            rot(self.x, self.y),
            rot(self.x + self.w, self.y),
            rot(self.x + self.w, self.y + self.h),
            rot(self.x, self.y + self.h),
        ]
    }

    pub fn center(&self) -> (f32, f32) {
        let c = self.corners();
        ((c[0].0 + c[2].0) / 2.0, (c[0].1 + c[2].1) / 2.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub name: String,
    pub rows: u8,
    pub cols: u8,
    pub keys: Vec<KeyGeom>,
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("keyboard definition is missing {0}")]
    Missing(&'static str),
    #[error("bad layout entry: {0}")]
    Bad(String),
}

/// KLE label-slot remapping by alignment (`a`), from kle-serial.
const LABEL_MAP: [[i8; 12]; 8] = [
    [0, 6, 2, 8, 9, 11, 3, 5, 1, 4, 7, 10],
    [1, 7, -1, -1, 9, 11, 4, -1, -1, -1, -1, 10],
    [3, -1, 5, -1, 9, 11, -1, -1, 4, -1, -1, 10],
    [4, -1, -1, -1, 9, 11, -1, -1, -1, -1, -1, 10],
    [0, 6, 2, 8, 10, -1, 3, 5, 1, 4, 7, -1],
    [1, 7, -1, -1, 10, -1, 4, -1, -1, -1, -1, -1],
    [3, -1, 5, -1, 10, -1, -1, -1, 4, -1, -1, -1],
    [4, -1, -1, -1, 10, -1, -1, -1, -1, -1, -1, -1],
];

fn reorder_labels(text: &str, align: usize) -> [Option<String>; 12] {
    let mut out: [Option<String>; 12] = Default::default();
    for (i, label) in text.split('\n').enumerate().take(12) {
        let slot = LABEL_MAP[align.min(7)][i];
        if slot >= 0 && !label.is_empty() {
            out[slot as usize] = Some(label.to_owned());
        }
    }
    out
}

fn parse_row_col(s: &str) -> Option<(u8, u8)> {
    let (r, c) = s.split_once(',')?;
    Some((r.trim().parse().ok()?, c.trim().parse().ok()?))
}

impl Layout {
    pub fn from_definition(def: &Value) -> Result<Layout, LayoutError> {
        let dim = |ptr: &'static str| -> Result<u8, LayoutError> {
            def.pointer(ptr)
                .and_then(Value::as_u64)
                .and_then(|v| u8::try_from(v).ok())
                .ok_or(LayoutError::Missing(ptr))
        };
        let rows = dim("/matrix/rows")?;
        let cols = dim("/matrix/cols")?;
        let kle = def
            .pointer("/layouts/keymap")
            .and_then(Value::as_array)
            .ok_or(LayoutError::Missing("/layouts/keymap"))?;
        let name = def.get("name").and_then(Value::as_str).unwrap_or("keyboard").to_owned();

        let mut keys = Vec::new();
        let (mut x, mut y, mut w, mut h) = (0.0f32, 0.0f32, 1.0f32, 1.0f32);
        let (mut angle, mut rx, mut ry) = (0.0f32, 0.0f32, 0.0f32);
        let (mut align, mut decal) = (4usize, false);

        for row in kle {
            let Some(items) = row.as_array() else { continue }; // metadata object
            for item in items {
                match item {
                    Value::Object(props) => {
                        let num = |k: &str| props.get(k).and_then(Value::as_f64).map(|v| v as f32);
                        if let Some(v) = num("r") {
                            angle = v;
                        }
                        if let Some(v) = num("rx") {
                            rx = v;
                            (x, y) = (rx, ry);
                        }
                        if let Some(v) = num("ry") {
                            ry = v;
                            (x, y) = (rx, ry);
                        }
                        if let Some(v) = props.get("a").and_then(Value::as_u64) {
                            align = v as usize;
                        }
                        if let Some(v) = num("x") {
                            x += v;
                        }
                        if let Some(v) = num("y") {
                            y += v;
                        }
                        if let Some(v) = num("w") {
                            w = v;
                        }
                        if let Some(v) = num("h") {
                            h = v;
                        }
                        if let Some(v) = props.get("d").and_then(Value::as_bool) {
                            decal = v;
                        }
                    }
                    Value::String(text) => {
                        let labels = reorder_labels(text, align);
                        let encoder = labels[4].as_deref() == Some("e");
                        let default_option = labels[8].as_deref().is_none_or(|opt| opt.ends_with(",0"));
                        let rc = labels[0].as_deref().and_then(parse_row_col);
                        if let (false, false, true, Some((r, c))) = (encoder, decal, default_option, rc) {
                            if r >= rows || c >= cols {
                                return Err(LayoutError::Bad(format!("key {r},{c} is outside the {rows}x{cols} matrix")));
                            }
                            keys.push(KeyGeom { row: r, col: c, x, y, w, h, angle, rx, ry });
                        }
                        x += w;
                        (w, h, decal) = (1.0, 1.0, false);
                    }
                    other => return Err(LayoutError::Bad(other.to_string())),
                }
            }
            y += 1.0;
            x = rx;
        }
        if keys.is_empty() {
            return Err(LayoutError::Missing("keys with row,col labels"));
        }
        Ok(Layout { name, rows, cols, keys })
    }

    /// (min_x, min_y, max_x, max_y) over all rotated key corners, in key units.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for key in &self.keys {
            for (px, py) in key.corners() {
                b = (b.0.min(px), b.1.min(py), b.2.max(px), b.3.max(py));
            }
        }
        b
    }

    pub fn key(&self, row: u8, col: u8) -> Option<&KeyGeom> {
        self.keys.iter().find(|k| k.row == row && k.col == col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(keymap: Value, rows: u8, cols: u8) -> Value {
        json!({ "name": "Test", "matrix": { "rows": rows, "cols": cols }, "layouts": { "keymap": keymap } })
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn places_keys_left_to_right_and_row_by_row() {
        let layout = Layout::from_definition(&def(json!([["0,0", "0,1"], [{ "w": 2 }, "1,0"]]), 2, 2)).unwrap();
        let k: Vec<_> = layout.keys.iter().map(|k| (k.row, k.col, k.x, k.y, k.w)).collect();
        assert_eq!(k, vec![(0, 0, 0.0, 0.0, 1.0), (0, 1, 1.0, 0.0, 1.0), (1, 0, 0.0, 1.0, 2.0)]);
        assert_eq!(layout.name, "Test");
    }

    #[test]
    fn handles_lily58_style_offsets() {
        let km = json!([[{ "x": 3.5 }, "0,3", { "x": 8.5 }, "5,3"], [{ "y": -0.875, "x": 2.5 }, "0,2"]]);
        let layout = Layout::from_definition(&def(km, 10, 6)).unwrap();
        let (a, b, c) = (layout.key(0, 3).unwrap(), layout.key(5, 3).unwrap(), layout.key(0, 2).unwrap());
        assert!(close(a.x, 3.5) && close(a.y, 0.0));
        assert!(close(b.x, 13.0) && close(b.y, 0.0));
        assert!(close(c.x, 2.5) && close(c.y, 0.125));
    }

    #[test]
    fn rotation_about_rx_ry() {
        let km = json!([[{ "r": 15, "rx": 4, "ry": 3, "y": -1, "x": 1 }, "0,0"]]);
        let layout = Layout::from_definition(&def(km, 1, 1)).unwrap();
        let k = &layout.keys[0];
        assert!(close(k.x, 5.0) && close(k.y, 2.0) && close(k.angle, 15.0));
        let (x, y) = k.corners()[0];
        assert!(close(x, 5.2247) && close(y, 2.2929), "got {x},{y}");
    }

    #[test]
    fn skips_metadata_decals_encoders_and_non_default_options() {
        let km = json!([
            { "name": "meta" },
            ["0,0", { "d": true }, "", "0,1\n\n\n\n\n\n\n\n\ne", "0,2\n\n\n1,1"],
            ["1,0"]
        ]);
        let layout = Layout::from_definition(&def(km, 2, 3)).unwrap();
        let rc: Vec<_> = layout.keys.iter().map(|k| (k.row, k.col)).collect();
        assert_eq!(rc, vec![(0, 0), (1, 0)]);
    }

    #[test]
    fn rejects_keys_outside_the_matrix_and_missing_fields() {
        assert!(Layout::from_definition(&def(json!([["2,0"]]), 2, 2)).is_err());
        assert!(Layout::from_definition(&json!({ "layouts": { "keymap": [] } })).is_err());
    }

    #[test]
    fn bounds_cover_all_corners() {
        let layout = Layout::from_definition(&def(json!([["0,0", { "w": 1.5 }, "0,1"]]), 1, 2)).unwrap();
        assert_eq!(layout.bounds(), (0.0, 0.0, 2.5, 1.0));
    }

    #[test]
    fn parses_the_captured_lily58_definition() {
        let def: Value = serde_json::from_str(include_str!("../tests/fixtures/lily58-definition.json")).unwrap();
        let layout = Layout::from_definition(&def).unwrap();
        assert_eq!((layout.rows, layout.cols), (10, 6));
        assert_eq!(layout.keys.len(), 60);
        let (min_x, min_y, max_x, max_y) = layout.bounds();
        assert!(max_x - min_x > 10.0 && max_y - min_y > 3.0, "split board should be wide: {:?}", layout.bounds());
    }
}
