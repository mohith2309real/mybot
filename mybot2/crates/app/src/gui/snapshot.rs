//! Repeatable screenshots of the window, for the README and for checking a
//! change by eye without clicking through it:
//!
//! ```text
//! MYBOT_SNAPSHOT=out.png MYBOT_SNAPSHOT_VIEW=skills mybot2
//! ```
//!
//! opens the app, lets it settle, writes one frame as a PNG and quits. Views:
//! `chat` (default), `computer`, `needs-you`, `new-teammate`, `skills`,
//! `imported`, `routines`, `logins`, `settings`. A pending sign-in request in
//! the data shows on any view. `MYBOT_SNAPSHOT_BOT=<name>` picks the
//! conversation. Inert unless `MYBOT_SNAPSHOT` is set. Pair it with a
//! throwaway `MYBOT_HOME` so it never shows your real data.

use std::path::PathBuf;

pub struct Snapshot {
    pub path: PathBuf,
    pub view: String,
    pub bot: Option<String>,
    frames: u32,
    requested: bool,
}

impl Snapshot {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("MYBOT_SNAPSHOT").filter(|p| !p.is_empty())?;
        Some(Self {
            path: PathBuf::from(path),
            view: std::env::var("MYBOT_SNAPSHOT_VIEW").unwrap_or_else(|_| "chat".into()),
            bot: std::env::var("MYBOT_SNAPSHOT_BOT").ok().filter(|b| !b.is_empty()),
            frames: 0,
            requested: false,
        })
    }

    /// Call once per frame. Returns true once the file is written.
    pub fn tick(&mut self, ctx: &egui::Context) -> bool {
        self.frames += 1;
        ctx.request_repaint();
        // Enough frames for fonts, layout and the first job polls to settle.
        if !self.requested && self.frames >= 40 {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = shot else { return false };
        let [w, h] = image.size;
        let mut rgba = Vec::with_capacity(w * h * 4);
        for p in &image.pixels {
            rgba.extend_from_slice(&p.to_srgba_unmultiplied());
        }
        match std::fs::write(&self.path, png(w as u32, h as u32, &rgba)) {
            Ok(()) => eprintln!("snapshot: wrote {} ({w}×{h})", self.path.display()),
            Err(e) => eprintln!("snapshot: could not write {}: {e}", self.path.display()),
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        true
    }
}

/// A minimal PNG writer: RGBA8, stored (uncompressed) deflate blocks. Big
/// files, no dependencies; re-encode afterwards if the size matters.
pub fn png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    }

    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for row in rgba.chunks(w as usize * 4) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = b.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut bsum) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65_521;
        bsum = (bsum + a) % 65_521;
    }
    z.extend_from_slice(&((bsum << 16) | a).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn png_has_a_valid_header_and_size() {
        let px = vec![255u8; 3 * 2 * 4];
        let f = super::png(3, 2, &px);
        assert_eq!(&f[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&f[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(f[16..20].try_into().unwrap()), 3);
        assert_eq!(u32::from_be_bytes(f[20..24].try_into().unwrap()), 2);
        assert_eq!(&f[f.len() - 8..f.len() - 4], b"IEND");
    }
}
