//! Regular lat/lon grids: ESRI ASCII parsing, compact binary storage, area means,
//! and rendering to Web Mercator PNG tiles for MapLibre.

use anyhow::{Context, Result, bail};
use std::f64::consts::PI;
use std::io::Write;
use std::path::Path;

/// Row 0 is the northernmost row (ESRI ASCII order).
#[derive(Clone, Debug)]
pub struct Grid {
    pub ncols: usize,
    pub nrows: usize,
    /// Centre of the westernmost column.
    pub lon0: f64,
    /// Centre of the northernmost row.
    pub lat_top: f64,
    pub cell: f64,
    pub data: Vec<f32>,
}


impl Grid {
    pub fn parse_esri_ascii(text: &str) -> Result<Grid> {
        let mut header = std::collections::HashMap::new();
        let mut lines = text.lines();
        let mut first_data = None;
        for line in lines.by_ref() {
            let mut it = line.split_whitespace();
            let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
            if k.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                header.insert(k.to_ascii_lowercase(), v.parse::<f64>()?);
            } else {
                first_data = Some(line);
                break;
            }
        }
        let h = |k: &str| header.get(k).copied().with_context(|| format!("missing header {k}"));
        let (ncols, nrows) = (h("ncols")? as usize, h("nrows")? as usize);
        let cell = h("cellsize")?;
        let nodata = h("nodata_value").unwrap_or(-9999.0) as f32;
        let lon0 = header.get("xllcenter").copied().or(header.get("xllcorner").map(|x| x + cell / 2.0)).context("xll")?;
        let lat0 = header.get("yllcenter").copied().or(header.get("yllcorner").map(|y| y + cell / 2.0)).context("yll")?;
        let mut data = Vec::with_capacity(ncols * nrows);
        for line in first_data.into_iter().chain(lines) {
            for tok in line.split_whitespace() {
                let v: f32 = tok.parse()?;
                data.push(if (v - nodata).abs() < 1e-4 { f32::NAN } else { v });
            }
        }
        if data.len() != ncols * nrows {
            bail!("expected {} values, got {}", ncols * nrows, data.len());
        }
        Ok(Grid { ncols, nrows, lon0, lat_top: lat0 + (nrows as f64 - 1.0) * cell, cell, data })
    }

    pub fn get(&self, r: usize, c: usize) -> f32 {
        self.data[r * self.ncols + c]
    }

    fn lat(&self, r: usize) -> f64 {
        self.lat_top - r as f64 * self.cell
    }

    fn lon(&self, c: usize) -> f64 {
        self.lon0 + c as f64 * self.cell
    }

    /// Nearest cell value; lon may be in any range.
    pub fn sample_nearest(&self, lat: f64, lon: f64) -> Option<f32> {
        let r = ((self.lat_top - lat) / self.cell).round();
        let c = ((wrap180(lon) - self.lon0) / self.cell).round().rem_euclid(self.ncols as f64);
        if r < 0.0 || r >= self.nrows as f64 {
            return None;
        }
        let v = self.get(r as usize, c as usize);
        (!v.is_nan()).then_some(v)
    }

    /// Bilinear interpolation that ignores missing neighbours (land).
    fn sample_bilinear(&self, lat: f64, lon: f64) -> Option<f32> {
        let fr = (self.lat_top - lat) / self.cell;
        let fc = (wrap180(lon) - self.lon0) / self.cell;
        let (r0, c0) = (fr.floor(), fc.floor());
        let (dr, dc) = (fr - r0, fc - c0);
        let mut acc = 0.0;
        let mut wsum = 0.0;
        for (rr, wr) in [(r0, 1.0 - dr), (r0 + 1.0, dr)] {
            if rr < 0.0 || rr >= self.nrows as f64 {
                continue;
            }
            for (cc, wc) in [(c0, 1.0 - dc), (c0 + 1.0, dc)] {
                let c = cc.rem_euclid(self.ncols as f64) as usize;
                let v = self.get(rr as usize, c);
                let w = wr * wc;
                if !v.is_nan() && w > 0.0 {
                    acc += v as f64 * w;
                    wsum += w;
                }
            }
        }
        (wsum > 0.25).then(|| (acc / wsum) as f32)
    }

    /// Cos-latitude weighted mean over a box (cells whose centres fall inside).
    pub fn area_mean(&self, lat_min: f64, lat_max: f64, lon_min: f64, lon_max: f64) -> Option<f64> {
        let mut acc = 0.0;
        let mut wsum = 0.0;
        for r in 0..self.nrows {
            let lat = self.lat(r);
            if lat < lat_min || lat > lat_max {
                continue;
            }
            let w = lat.to_radians().cos();
            for c in 0..self.ncols {
                let lon = self.lon(c);
                if lon < lon_min || lon > lon_max {
                    continue;
                }
                let v = self.get(r, c);
                if !v.is_nan() {
                    acc += v as f64 * w;
                    wsum += w;
                }
            }
        }
        (wsum > 0.0).then(|| acc / wsum)
    }

    /// Mean over 5°S–5°N for each column between lon_from and lon_to (0–360 convention),
    /// returned as (lon in 0–360, value).
    pub fn equatorial_band(&self, lon_from: f64, lon_to: f64) -> Vec<(f64, Option<f64>)> {
        let rows: Vec<usize> = (0..self.nrows).filter(|&r| self.lat(r).abs() <= 5.0).collect();
        let mut cols: Vec<(f64, usize)> = (0..self.ncols)
            .map(|c| (self.lon(c).rem_euclid(360.0), c))
            .filter(|(l, _)| *l >= lon_from && *l <= lon_to)
            .collect();
        cols.sort_by(|a, b| a.0.total_cmp(&b.0));
        cols.into_iter()
            .map(|(lon, c)| {
                let vals: Vec<f64> = rows.iter().map(|&r| self.get(r, c)).filter(|v| !v.is_nan()).map(f64::from).collect();
                let mean = (!vals.is_empty()).then(|| vals.iter().sum::<f64>() / vals.len() as f64);
                (lon, mean)
            })
            .collect()
    }

    // ------------------------------------------------------------ binary storage
    // Layout: b"GRD1", ncols u32, nrows u32, lon0 f32, lat_top f32, cell f32, then i16 centi-units (LE).

    pub fn save(&self, path: &Path) -> Result<()> {
        let mut buf = Vec::with_capacity(24 + self.data.len() * 2);
        buf.extend_from_slice(b"GRD1");
        buf.extend_from_slice(&(self.ncols as u32).to_le_bytes());
        buf.extend_from_slice(&(self.nrows as u32).to_le_bytes());
        buf.extend_from_slice(&(self.lon0 as f32).to_le_bytes());
        buf.extend_from_slice(&(self.lat_top as f32).to_le_bytes());
        buf.extend_from_slice(&(self.cell as f32).to_le_bytes());
        for v in &self.data {
            let q = if v.is_nan() { i16::MIN } else { (v * 100.0).round().clamp(-32000.0, 32000.0) as i16 };
            buf.extend_from_slice(&q.to_le_bytes());
        }
        write_atomic(path, &buf)
    }

    pub fn load(path: &Path) -> Result<Grid> {
        let b = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        if b.len() < 24 || &b[..4] != b"GRD1" {
            bail!("{} is not a grid file", path.display());
        }
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let f32_at = |i: usize| f32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as f64;
        let (ncols, nrows) = (u32_at(4) as usize, u32_at(8) as usize);
        if b.len() != 24 + ncols * nrows * 2 {
            bail!("{} has wrong size", path.display());
        }
        let data = b[24..]
            .chunks_exact(2)
            .map(|c| {
                let q = i16::from_le_bytes([c[0], c[1]]);
                if q == i16::MIN { f32::NAN } else { q as f32 / 100.0 }
            })
            .collect();
        Ok(Grid { ncols, nrows, lon0: f32_at(12), lat_top: f32_at(16), cell: f32_at(20), data })
    }

    // ------------------------------------------------------------ rendering

    /// Renders one 256 px Web Mercator (XYZ) tile as an indexed PNG.
    pub fn render_tile(&self, z: u32, x: u32, y: u32, scale: &ColorScale) -> Result<Vec<u8>> {
        const SIZE: u32 = 256;
        if z > 12 || x >= (1 << z) || y >= (1 << z) {
            bail!("tile out of range");
        }
        let n = (1u32 << z) as f64;
        let (lo, hi) = scale.range();
        let mut pixels = vec![0u8; (SIZE * SIZE) as usize];
        for py in 0..SIZE {
            let gy = (y as f64 + (py as f64 + 0.5) / SIZE as f64) / n;
            let lat = (PI * (1.0 - 2.0 * gy)).sinh().atan().to_degrees();
            for px in 0..SIZE {
                let gx = (x as f64 + (px as f64 + 0.5) / SIZE as f64) / n;
                if let Some(v) = self.sample_bilinear(lat, gx * 360.0 - 180.0) {
                    let t = ((v as f64 - lo) / (hi - lo)).clamp(0.0, 1.0);
                    pixels[(py * SIZE + px) as usize] = 1 + (t * 254.0).round() as u8;
                }
            }
        }
        encode_indexed(SIZE, SIZE, &pixels, scale)
    }
}

/// Indexed PNG: entry 0 is transparent (missing), entries 1..=255 span the scale's
/// range in equal steps (~0.04 °C for ±5 °C), which keeps tiles small.
fn encode_indexed(w: u32, h: u32, pixels: &[u8], scale: &ColorScale) -> Result<Vec<u8>> {
    let (lo, hi) = scale.range();
    let mut palette = vec![0u8; 3];
    for i in 0..255 {
        palette.extend_from_slice(&scale.color(lo + (hi - lo) * i as f64 / 254.0));
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(palette);
        enc.set_trns(vec![0u8]);
        enc.set_compression(png::Compression::High);
        let mut writer = enc.write_header()?;
        writer.write_image_data(pixels)?;
    }
    Ok(out)
}

fn wrap180(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Piecewise-linear colour scale.
pub struct ColorScale {
    pub stops: &'static [(f64, [u8; 3])],
}

/// Diverging scale for SST anomalies (°C), ColorBrewer RdBu, fixed at ±5 °C.
/// Kept in sync with SST_STOPS in static/js/common.js (legend).
pub const SST_ANOMALY: ColorScale = ColorScale {
    stops: &[
        (-5.0, [5, 48, 97]),
        (-3.0, [33, 102, 172]),
        (-2.0, [67, 147, 195]),
        (-1.0, [146, 197, 222]),
        (-0.5, [209, 229, 240]),
        (0.0, [247, 247, 247]),
        (0.5, [253, 219, 199]),
        (1.0, [244, 165, 130]),
        (2.0, [214, 96, 77]),
        (3.0, [178, 24, 43]),
        (5.0, [103, 0, 31]),
    ],
};

impl ColorScale {
    pub fn range(&self) -> (f64, f64) {
        (self.stops[0].0, self.stops[self.stops.len() - 1].0)
    }

    pub fn color(&self, v: f64) -> [u8; 3] {
        let s = self.stops;
        if v <= s[0].0 {
            return s[0].1;
        }
        for pair in s.windows(2) {
            let ((v0, c0), (v1, c1)) = (pair[0], pair[1]);
            if v <= v1 {
                let t = (v - v0) / (v1 - v0);
                let mix = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round() as u8;
                return [mix(c0[0], c1[0]), mix(c0[1], c1[1]), mix(c0[2], c1[2])];
            }
        }
        s[s.len() - 1].1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Grid {
        let text = std::fs::read_to_string(format!("{}/tests/fixtures/oisst_nrt.asc", env!("CARGO_MANIFEST_DIR"))).unwrap();
        Grid::parse_esri_ascii(&text).unwrap()
    }

    #[test]
    fn parses_and_roundtrips() {
        let g = fixture();
        assert_eq!((g.ncols, g.nrows), (360, 180));
        assert!((g.lat_top - 89.125).abs() < 1e-6);
        // Land (central Brazil) is missing, open ocean is not.
        assert!(g.sample_nearest(-10.0, -50.0).is_none());
        assert!(g.sample_nearest(0.0, -140.0).is_some());
        let dir = std::env::temp_dir().join(format!("elnino-grid-{}", std::process::id()));
        let path = dir.join("g.bin");
        g.save(&path).unwrap();
        let h = Grid::load(&path).unwrap();
        assert_eq!(h.data.len(), g.data.len());
        assert!((h.get(90, 40) - g.get(90, 40)).abs() < 0.006 || g.get(90, 40).is_nan());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn nino_regions_are_warm() {
        let g = fixture();
        let n34 = g.area_mean(-5.0, 5.0, -170.0, -120.0).unwrap();
        let n12 = g.area_mean(-10.0, 0.0, -90.0, -80.0).unwrap();
        // Strong El Niño in the fixture (Oct 2026), anomalies vs 1971–2000.
        assert!(n34 > 2.0, "{n34}");
        assert!(n12 > 2.0, "{n12}");
        let band = g.equatorial_band(120.0, 280.0);
        assert_eq!(band.len(), 160);
    }

    #[test]
    fn tiles() {
        let g = fixture();
        let png = g.render_tile(1, 0, 0, &SST_ANOMALY).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert!(g.render_tile(1, 2, 0, &SST_ANOMALY).is_err());
    }

    #[test]
    fn colors() {
        assert_eq!(SST_ANOMALY.color(0.0), [247, 247, 247]);
        assert_eq!(SST_ANOMALY.color(10.0), [103, 0, 31]);
        assert_eq!(SST_ANOMALY.color(-0.25), [228, 238, 244]);
    }
}
