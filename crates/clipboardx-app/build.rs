fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(target_os = "windows")]
    windows_icon::embed();
}

#[cfg(target_os = "windows")]
mod windows_icon {
    use std::env;
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;

    use image::{ImageFormat, RgbaImage};

    pub fn embed() {
        let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
            return;
        };
        let ico_path = out_dir.join("clipboardx.ico");
        let ico_bytes = build_multi_size_ico(&[16, 32, 48, 64, 128, 256]);
        if fs::write(&ico_path, ico_bytes).is_err() {
            return;
        };

        // Also write a copy to assets/brand/clipboardx.ico when building inside the workspace
        // so Windows installer packaging (NSIS) can reference the exact same icon file.
        if let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from) {
            let brand_ico = manifest_dir
                .join("..")
                .join("..")
                .join("assets")
                .join("brand")
                .join("clipboardx.ico");
            if !brand_ico.exists() {
                let _ = fs::write(
                    &brand_ico,
                    build_multi_size_ico(&[16, 32, 48, 64, 128, 256]),
                );
            }
        }

        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico_path.to_string_lossy().as_ref());
        res.set("ProductName", "ClipboardX");
        res.set(
            "FileDescription",
            "ClipboardX - Offline Clipboard Manager & Encrypted Vault",
        );
        res.set("LegalCopyright", "Copyright (c) ClipboardX Contributors");
        let _ = res.compile();
    }

    fn build_multi_size_ico(sizes: &[u32]) -> Vec<u8> {
        let mut png_blobs: Vec<(u32, Vec<u8>)> = Vec::with_capacity(sizes.len());
        for &sz in sizes {
            let rgba = render_brand_icon_rgba(sz as usize);
            let Some(img) = RgbaImage::from_raw(sz, sz, rgba) else {
                continue;
            };
            let mut buf = Cursor::new(Vec::new());
            if img.write_to(&mut buf, ImageFormat::Png).is_ok() {
                png_blobs.push((sz, buf.into_inner()));
            }
        }

        let count = png_blobs.len() as u16;
        let mut out = Vec::new();
        // ICONDIR header (6 bytes)
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());

        let mut offset = 6u32 + (u32::from(count) * 16);
        for (sz, blob) in &png_blobs {
            let dim_byte = if *sz >= 256 { 0u8 } else { *sz as u8 };
            let bytes_in_res = blob.len() as u32;
            out.push(dim_byte);
            out.push(dim_byte);
            out.push(0);
            out.push(0);
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&32u16.to_le_bytes());
            out.extend_from_slice(&bytes_in_res.to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
            offset += bytes_in_res;
        }

        for (_, blob) in &png_blobs {
            out.extend_from_slice(blob);
        }
        out
    }

    fn render_brand_icon_rgba(size: usize) -> Vec<u8> {
        const SUB: usize = 4;
        let mut rgba = vec![0u8; size * size * 4];
        let scale = 32.0 / (size as f32);

        for py in 0..size {
            for px in 0..size {
                let mut r_acc = 0.0_f32;
                let mut g_acc = 0.0_f32;
                let mut b_acc = 0.0_f32;
                let mut a_acc = 0.0_f32;

                for sy in 0..SUB {
                    for sx in 0..SUB {
                        let x = (px as f32 + (sx as f32 + 0.5) / SUB as f32) * scale;
                        let y = (py as f32 + (sy as f32 + 0.5) / SUB as f32) * scale;

                        if !inside_rounded_rect(x, y, 1.0, 1.0, 30.0, 30.0, 6.8) {
                            continue;
                        }

                        let grad_t = (((x - 1.0) * 0.45 + (y - 1.0) * 0.55) / 30.0).clamp(0.0, 1.0);
                        let mut r = lerp_u8(0x1E, 0x1D, grad_t);
                        let mut g = lerp_u8(0x29, 0x4E, grad_t);
                        let mut b = lerp_u8(0x3B, 0xD8, grad_t);

                        if inside_rounded_rect(x, y, 10.2, 8.2, 13.6, 16.8, 3.2) {
                            r = r * 0.72 + 147.0 * 0.28;
                            g = g * 0.72 + 197.0 * 0.28;
                            b = b * 0.72 + 253.0 * 0.28;
                        }

                        if inside_rounded_rect(x, y, 8.0, 7.2, 14.6, 18.0, 3.4) {
                            let card_t = ((y - 7.2) / 18.0).clamp(0.0, 1.0);
                            r = lerp_u8(0xFF, 0xE2, card_t);
                            g = lerp_u8(0xFF, 0xEE, card_t);
                            b = lerp_u8(0xFF, 0xFF, card_t);

                            let d_main = dist_to_segment(x, y, 12.1, 13.4, 18.5, 19.8);
                            let d_accent = dist_to_segment(x, y, 18.5, 13.4, 12.1, 19.8);
                            if d_accent <= 1.25 {
                                r = 0x38 as f32;
                                g = 0xBD as f32;
                                b = 0xF8 as f32;
                            }
                            if d_main <= 1.25 {
                                r = 0x25 as f32;
                                g = 0x63 as f32;
                                b = 0xEB as f32;
                            }
                        }

                        if inside_rounded_rect(x, y, 11.6, 5.2, 7.4, 3.5, 1.75) {
                            r = 0x3B as f32;
                            g = 0x82 as f32;
                            b = 0xF6 as f32;
                        }

                        r_acc += r;
                        g_acc += g;
                        b_acc += b;
                        a_acc += 255.0;
                    }
                }

                let samples = (SUB * SUB) as f32;
                let idx = (py * size + px) * 4;
                if a_acc > 0.0 {
                    let cov = a_acc / (samples * 255.0);
                    let inv_cov = 1.0 / (cov * samples);
                    rgba[idx] = (r_acc * inv_cov).clamp(0.0, 255.0).round() as u8;
                    rgba[idx + 1] = (g_acc * inv_cov).clamp(0.0, 255.0).round() as u8;
                    rgba[idx + 2] = (b_acc * inv_cov).clamp(0.0, 255.0).round() as u8;
                    rgba[idx + 3] = (a_acc / samples).clamp(0.0, 255.0).round() as u8;
                }
            }
        }
        rgba
    }

    fn lerp_u8(a: u8, b: u8, t: f32) -> f32 {
        let fa = f32::from(a);
        let fb = f32::from(b);
        fa + (fb - fa) * t
    }

    fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
        let abx = bx - ax;
        let aby = by - ay;
        let len_sq = abx * abx + aby * aby;
        let t = if len_sq <= f32::EPSILON {
            0.0
        } else {
            (((px - ax) * abx + (py - ay) * aby) / len_sq).clamp(0.0, 1.0)
        };
        let dx = px - (ax + abx * t);
        let dy = py - (ay + aby * t);
        (dx * dx + dy * dy).sqrt()
    }

    fn inside_rounded_rect(x: f32, y: f32, left: f32, top: f32, w: f32, h: f32, r: f32) -> bool {
        let right = left + w;
        let bottom = top + h;
        if x < left || x > right || y < top || y > bottom {
            return false;
        }
        let rr = r.min(w * 0.5).min(h * 0.5);
        let dx = if x < left + rr {
            (left + rr) - x
        } else if x > right - rr {
            x - (right - rr)
        } else {
            0.0
        };
        let dy = if y < top + rr {
            (top + rr) - y
        } else if y > bottom - rr {
            y - (bottom - rr)
        } else {
            0.0
        };
        dx * dx + dy * dy <= rr * rr
    }
}
