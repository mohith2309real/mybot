//! A VNC (RFB 3.3/3.7/3.8) client for the native live view.
//!
//! The container serves each desktop through websockify, so the usual route is
//! RFB over a WebSocket (`ws://127.0.0.1:<view port>/websockify`); plain TCP
//! works too. Encodings: Raw, CopyRect and DesktopSize — everything is on
//! loopback, so bandwidth-saving codecs would only add code.
//!
//! The framebuffer is kept as RGBA, ready to hand to the UI as a texture.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use des::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[derive(Debug, Clone, Default)]
pub struct Framebuffer {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
    pub name: String,
    /// Bumped on every change, so the UI re-uploads only when needed.
    pub generation: u64,
}

impl Framebuffer {
    fn resize(&mut self, w: u16, h: u16) {
        self.width = w;
        self.height = h;
        self.rgba = vec![0; w as usize * h as usize * 4];
        for px in self.rgba.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Button mask: bit 0 left, 1 middle, 2 right, 3/4 wheel up/down.
    Pointer { x: u16, y: u16, buttons: u8 },
    Key { keysym: u32, down: bool },
    Clipboard(String),
}

#[derive(Clone)]
pub struct LiveView {
    pub fb: Arc<Mutex<Framebuffer>>,
    input: mpsc::UnboundedSender<Input>,
    connected: Arc<AtomicBool>,
    pub error: Arc<Mutex<Option<String>>>,
    pub clipboard: Arc<Mutex<Option<String>>>,
}

impl std::fmt::Debug for LiveView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveView").field("connected", &self.is_connected()).finish()
    }
}

impl LiveView {
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub fn send(&self, i: Input) {
        let _ = self.input.send(i);
    }

    /// Type a string: one key press/release per character.
    pub fn type_text(&self, text: &str) {
        for c in text.chars() {
            let ks = char_keysym(c);
            self.send(Input::Key { keysym: ks, down: true });
            self.send(Input::Key { keysym: ks, down: false });
        }
    }

    pub async fn connect_ws(url: &str, password: Option<&str>) -> Result<Self, String> {
        let mut req = url.into_client_request().map_err(|e| e.to_string())?;
        req.headers_mut().insert("Sec-WebSocket-Protocol", "binary".parse().unwrap());
        let (ws, _) = tokio_tungstenite::connect_async(req).await.map_err(|e| format!("live view: {e}"))?;
        // Bridge WebSocket frames to a byte stream: websockify splits RFB
        // messages across frames at arbitrary points.
        let (local, remote) = tokio::io::duplex(1 << 20);
        let (mut rd, mut wr) = tokio::io::split(remote);
        let (mut sink, mut stream) = ws.split();
        tokio::spawn(async move {
            while let Some(Ok(m)) = stream.next().await {
                let bytes = match m {
                    WsMessage::Binary(b) => b.to_vec(),
                    WsMessage::Text(t) => t.as_bytes().to_vec(),
                    WsMessage::Close(_) => break,
                    _ => continue,
                };
                if wr.write_all(&bytes).await.is_err() {
                    break;
                }
            }
            let _ = wr.shutdown().await;
        });
        tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match rd.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if sink.send(WsMessage::Binary(buf[..n].to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = sink.close().await;
        });
        Self::start(local, password).await
    }

    pub async fn connect_tcp(addr: &str, password: Option<&str>) -> Result<Self, String> {
        let s = tokio::net::TcpStream::connect(addr).await.map_err(|e| format!("live view: {e}"))?;
        s.set_nodelay(true).ok();
        Self::start(s, password).await
    }

    async fn start<S>(mut s: S, password: Option<&str>) -> Result<Self, String>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let fb = handshake(&mut s, password).await?;
        let fb = Arc::new(Mutex::new(fb));
        let (input, mut input_rx) = mpsc::unbounded_channel::<Input>();
        let connected = Arc::new(AtomicBool::new(true));
        let error = Arc::new(Mutex::new(None));
        let clipboard = Arc::new(Mutex::new(None));
        let (mut rd, mut wr) = tokio::io::split(s);

        let (want_update, mut update_rx) = mpsc::unbounded_channel::<bool>();
        {
            let (w, h) = {
                let f = fb.lock().await;
                (f.width, f.height)
            };
            // 32bpp true colour laid out as R,G,B,X in memory, which is RGBA
            // once alpha is forced to 255.
            let mut pf = vec![0u8, 0, 0, 0, 32, 24, 0, 1];
            pf.extend_from_slice(&255u16.to_be_bytes());
            pf.extend_from_slice(&255u16.to_be_bytes());
            pf.extend_from_slice(&255u16.to_be_bytes());
            pf.extend_from_slice(&[0, 8, 16, 0, 0, 0]);
            wr.write_all(&pf).await.map_err(|e| e.to_string())?;
            let encs: [i32; 3] = [1, 0, -223];
            let mut se = vec![2u8, 0];
            se.extend_from_slice(&(encs.len() as u16).to_be_bytes());
            for e in encs {
                se.extend_from_slice(&e.to_be_bytes());
            }
            wr.write_all(&se).await.map_err(|e| e.to_string())?;
            wr.write_all(&update_request(false, w, h)).await.map_err(|e| e.to_string())?;
        }

        // Writer: input events and update requests.
        let fb_w = fb.clone();
        let conn_w = connected.clone();
        tokio::spawn(async move {
            loop {
                let msg = tokio::select! {
                    i = input_rx.recv() => match i {
                        Some(Input::Pointer { x, y, buttons }) => {
                            let mut m = vec![5u8, buttons];
                            m.extend_from_slice(&x.to_be_bytes());
                            m.extend_from_slice(&y.to_be_bytes());
                            m
                        }
                        Some(Input::Key { keysym, down }) => {
                            let mut m = vec![4u8, down as u8, 0, 0];
                            m.extend_from_slice(&keysym.to_be_bytes());
                            m
                        }
                        Some(Input::Clipboard(t)) => {
                            let bytes: Vec<u8> = t.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
                            let mut m = vec![6u8, 0, 0, 0];
                            m.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
                            m.extend_from_slice(&bytes);
                            m
                        }
                        None => break,
                    },
                    u = update_rx.recv() => match u {
                        Some(incremental) => {
                            let f = fb_w.lock().await;
                            update_request(incremental, f.width, f.height).to_vec()
                        }
                        None => break,
                    },
                };
                if wr.write_all(&msg).await.is_err() {
                    break;
                }
            }
            conn_w.store(false, Ordering::SeqCst);
        });

        // Reader: server messages.
        let fb_r = fb.clone();
        let conn_r = connected.clone();
        let err_r = error.clone();
        let clip_r = clipboard.clone();
        tokio::spawn(async move {
            let r = read_loop(&mut rd, &fb_r, &want_update, &clip_r).await;
            if let Err(e) = r {
                *err_r.lock().await = Some(e);
            }
            conn_r.store(false, Ordering::SeqCst);
        });

        Ok(Self { fb, input, connected, error, clipboard })
    }
}

fn update_request(incremental: bool, w: u16, h: u16) -> [u8; 10] {
    let mut m = [0u8; 10];
    m[0] = 3;
    m[1] = incremental as u8;
    m[6..8].copy_from_slice(&w.to_be_bytes());
    m[8..10].copy_from_slice(&h.to_be_bytes());
    m
}

async fn read_u8<R: AsyncRead + Unpin>(r: &mut R) -> Result<u8, String> {
    r.read_u8().await.map_err(|e| e.to_string())
}
async fn read_u16<R: AsyncRead + Unpin>(r: &mut R) -> Result<u16, String> {
    r.read_u16().await.map_err(|e| e.to_string())
}
async fn read_u32<R: AsyncRead + Unpin>(r: &mut R) -> Result<u32, String> {
    r.read_u32().await.map_err(|e| e.to_string())
}
async fn read_n<R: AsyncRead + Unpin>(r: &mut R, n: usize) -> Result<Vec<u8>, String> {
    let mut b = vec![0u8; n];
    r.read_exact(&mut b).await.map_err(|e| e.to_string())?;
    Ok(b)
}

/// VNC authentication: DES of the challenge, with the password's bits mirrored.
pub fn vnc_auth_response(password: &str, challenge: &[u8; 16]) -> [u8; 16] {
    let mut key = [0u8; 8];
    for (i, b) in password.bytes().take(8).enumerate() {
        key[i] = b.reverse_bits();
    }
    let cipher = des::Des::new(GenericArray::from_slice(&key));
    let mut out = [0u8; 16];
    for half in 0..2 {
        let mut block = GenericArray::clone_from_slice(&challenge[half * 8..half * 8 + 8]);
        cipher.encrypt_block(&mut block);
        out[half * 8..half * 8 + 8].copy_from_slice(&block);
    }
    out
}

async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S, password: Option<&str>) -> Result<Framebuffer, String> {
    let ver = read_n(s, 12).await?;
    let ver = String::from_utf8_lossy(&ver).to_string();
    if !ver.starts_with("RFB ") {
        return Err(format!("not a VNC server ({ver:?})"));
    }
    let minor: u32 = ver[8..11].parse().unwrap_or(3);
    let minor = if minor >= 8 { 8 } else if minor >= 7 { 7 } else { 3 };
    s.write_all(format!("RFB 003.00{minor}\n").as_bytes()).await.map_err(|e| e.to_string())?;

    let chosen: u32 = if minor == 3 {
        read_u32(s).await?
    } else {
        let n = read_u8(s).await?;
        if n == 0 {
            let len = read_u32(s).await? as usize;
            return Err(String::from_utf8_lossy(&read_n(s, len).await?).to_string());
        }
        let types = read_n(s, n as usize).await?;
        let pick = if types.contains(&1) {
            1
        } else if types.contains(&2) && password.is_some() {
            2
        } else if types.contains(&2) {
            return Err("this desktop needs a VNC password".into());
        } else {
            return Err(format!("no supported VNC security type (offered {types:?})"));
        };
        s.write_all(&[pick]).await.map_err(|e| e.to_string())?;
        pick as u32
    };
    match chosen {
        0 => {
            let len = read_u32(s).await? as usize;
            return Err(String::from_utf8_lossy(&read_n(s, len).await?).to_string());
        }
        1 => {}
        2 => {
            let mut challenge = [0u8; 16];
            s.read_exact(&mut challenge).await.map_err(|e| e.to_string())?;
            let resp = vnc_auth_response(password.unwrap_or(""), &challenge);
            s.write_all(&resp).await.map_err(|e| e.to_string())?;
        }
        other => return Err(format!("unsupported VNC security type {other}")),
    }
    if chosen == 2 || minor == 8 {
        let result = read_u32(s).await?;
        if result != 0 {
            let reason = if minor == 8 {
                let len = read_u32(s).await? as usize;
                String::from_utf8_lossy(&read_n(s, len).await?).to_string()
            } else {
                "authentication failed".into()
            };
            return Err(format!("VNC: {reason}"));
        }
    }

    s.write_all(&[1]).await.map_err(|e| e.to_string())?; // shared
    let w = read_u16(s).await?;
    let h = read_u16(s).await?;
    let _pf = read_n(s, 16).await?;
    let name_len = read_u32(s).await? as usize;
    let name = String::from_utf8_lossy(&read_n(s, name_len.min(4096)).await?).to_string();
    let mut fb = Framebuffer { name, ..Default::default() };
    fb.resize(w, h);
    Ok(fb)
}

async fn read_loop<R: AsyncRead + Unpin>(
    r: &mut R,
    fb: &Arc<Mutex<Framebuffer>>,
    want_update: &mpsc::UnboundedSender<bool>,
    clipboard: &Arc<Mutex<Option<String>>>,
) -> Result<(), String> {
    loop {
        match read_u8(r).await? {
            0 => {
                read_u8(r).await?;
                let n = read_u16(r).await?;
                for _ in 0..n {
                    let x = read_u16(r).await? as usize;
                    let y = read_u16(r).await? as usize;
                    let w = read_u16(r).await? as usize;
                    let h = read_u16(r).await? as usize;
                    let enc = read_u32(r).await? as i32;
                    match enc {
                        0 => {
                            let mut data = read_n(r, w * h * 4).await?;
                            for px in data.chunks_exact_mut(4) {
                                px[3] = 255;
                            }
                            let mut f = fb.lock().await;
                            let fw = f.width as usize;
                            for row in 0..h {
                                if y + row >= f.height as usize || x >= fw {
                                    break;
                                }
                                let cols = w.min(fw - x);
                                let dst = ((y + row) * fw + x) * 4;
                                let src = row * w * 4;
                                f.rgba[dst..dst + cols * 4].copy_from_slice(&data[src..src + cols * 4]);
                            }
                            f.generation += 1;
                        }
                        1 => {
                            let sx = read_u16(r).await? as usize;
                            let sy = read_u16(r).await? as usize;
                            let mut f = fb.lock().await;
                            let fw = f.width as usize;
                            let fh = f.height as usize;
                            let mut tmp = Vec::with_capacity(w * h * 4);
                            for row in 0..h {
                                if sy + row >= fh {
                                    break;
                                }
                                let cols = w.min(fw.saturating_sub(sx));
                                let s0 = ((sy + row) * fw + sx) * 4;
                                tmp.extend_from_slice(&f.rgba[s0..s0 + cols * 4]);
                                tmp.extend(std::iter::repeat_n(0u8, (w - cols) * 4));
                            }
                            for row in 0..h.min(tmp.len() / (w * 4).max(1)) {
                                if y + row >= fh || x >= fw {
                                    break;
                                }
                                let cols = w.min(fw - x);
                                let d0 = ((y + row) * fw + x) * 4;
                                f.rgba[d0..d0 + cols * 4].copy_from_slice(&tmp[row * w * 4..row * w * 4 + cols * 4]);
                            }
                            f.generation += 1;
                        }
                        -223 => {
                            let mut f = fb.lock().await;
                            f.resize(w as u16, h as u16);
                            f.generation += 1;
                        }
                        other => return Err(format!("the desktop sent an encoding this viewer does not speak ({other})")),
                    }
                }
                let _ = want_update.send(true);
            }
            1 => {
                read_u8(r).await?;
                read_u16(r).await?;
                let n = read_u16(r).await? as usize;
                read_n(r, n * 6).await?;
            }
            2 => {}
            3 => {
                read_n(r, 3).await?;
                let len = read_u32(r).await? as usize;
                let bytes = read_n(r, len.min(1 << 20)).await?;
                *clipboard.lock().await = Some(bytes.iter().map(|b| *b as char).collect());
            }
            other => return Err(format!("unexpected VNC message {other}")),
        }
    }
}

/// X11 keysym for a typed character.
pub fn char_keysym(c: char) -> u32 {
    let cp = c as u32;
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        '\u{8}' => 0xff08,
        _ if (0x20..=0x7e).contains(&cp) || (0xa0..=0xff).contains(&cp) => cp,
        _ => 0x0100_0000 | cp,
    }
}

/// X11 keysyms for named keys.
pub fn named_keysym(name: &str) -> Option<u32> {
    Some(match name {
        "Enter" => 0xff0d,
        "Tab" => 0xff09,
        "Backspace" => 0xff08,
        "Escape" => 0xff1b,
        "Delete" => 0xffff,
        "Insert" => 0xff63,
        "Home" => 0xff50,
        "End" => 0xff57,
        "PageUp" => 0xff55,
        "PageDown" => 0xff56,
        "ArrowLeft" => 0xff51,
        "ArrowUp" => 0xff52,
        "ArrowRight" => 0xff53,
        "ArrowDown" => 0xff54,
        "Shift" => 0xffe1,
        "Control" => 0xffe3,
        "Alt" => 0xffe9,
        "Meta" => 0xffe7,
        "Space" => 0x20,
        f if f.starts_with('F') && f[1..].parse::<u32>().is_ok_and(|n| (1..=12).contains(&n)) => 0xffbe + f[1..].parse::<u32>().unwrap() - 1,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn fake_server<S: AsyncRead + AsyncWrite + Unpin>(mut s: S, minor: u8, auth: Option<&str>) -> Vec<u8> {
        s.write_all(format!("RFB 003.00{minor}\n").as_bytes()).await.unwrap();
        let mut v = [0u8; 12];
        s.read_exact(&mut v).await.unwrap();
        if let Some(pw) = auth {
            s.write_all(&[1, 2]).await.unwrap();
            assert_eq!(s.read_u8().await.unwrap(), 2);
            let challenge = [7u8; 16];
            s.write_all(&challenge).await.unwrap();
            let mut resp = [0u8; 16];
            s.read_exact(&mut resp).await.unwrap();
            let ok = resp == vnc_auth_response(pw, &challenge);
            s.write_all(&(if ok { 0u32 } else { 1u32 }).to_be_bytes()).await.unwrap();
            if !ok {
                s.write_all(&6u32.to_be_bytes()).await.unwrap();
                s.write_all(b"denied").await.unwrap();
                return vec![];
            }
        } else {
            s.write_all(&[1, 1]).await.unwrap();
            assert_eq!(s.read_u8().await.unwrap(), 1);
            s.write_all(&0u32.to_be_bytes()).await.unwrap();
        }
        assert_eq!(s.read_u8().await.unwrap(), 1); // shared
        s.write_all(&4u16.to_be_bytes()).await.unwrap();
        s.write_all(&2u16.to_be_bytes()).await.unwrap();
        s.write_all(&[0u8; 16]).await.unwrap();
        s.write_all(&4u32.to_be_bytes()).await.unwrap();
        s.write_all(b"test").await.unwrap();
        // SetPixelFormat (20) + SetEncodings (4 + 3*4) + first update request (10)
        let mut setup = vec![0u8; 20 + 16 + 10];
        s.read_exact(&mut setup).await.unwrap();
        assert_eq!(setup[0], 0);
        assert_eq!(setup[20], 2);
        assert_eq!(setup[36], 3);
        // One raw rect: 2x2 at (1,0), all pure red (R,G,B,X).
        let mut fbu = vec![0u8, 0];
        fbu.extend_from_slice(&1u16.to_be_bytes());
        for v in [1u16, 0, 2, 2] {
            fbu.extend_from_slice(&v.to_be_bytes());
        }
        fbu.extend_from_slice(&0i32.to_be_bytes());
        for _ in 0..4 {
            fbu.extend_from_slice(&[255, 0, 0, 0]);
        }
        s.write_all(&fbu).await.unwrap();
        // The client asks for an incremental update, then sends a click.
        let mut req = [0u8; 10];
        s.read_exact(&mut req).await.unwrap();
        assert_eq!((req[0], req[1]), (3, 1));
        let mut ptr = [0u8; 6];
        s.read_exact(&mut ptr).await.unwrap();
        ptr.to_vec()
    }

    async fn wait_gen(v: &LiveView) {
        for _ in 0..100 {
            if v.fb.lock().await.generation > 0 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("no update arrived");
    }

    #[tokio::test]
    async fn tcp_none_auth_raw_and_pointer() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move { fake_server(l.accept().await.unwrap().0, 8, None).await });
        let view = LiveView::connect_tcp(&addr, None).await.unwrap();
        wait_gen(&view).await;
        {
            let f = view.fb.lock().await;
            assert_eq!((f.width, f.height, f.name.as_str()), (4, 2, "test"));
            assert_eq!(&f.rgba[4..8], &[255, 0, 0, 255], "pixel (1,0) is red and opaque");
            assert_eq!(&f.rgba[0..4], &[0, 0, 0, 255], "pixel (0,0) untouched");
        }
        view.send(Input::Pointer { x: 3, y: 1, buttons: 1 });
        assert_eq!(server.await.unwrap(), vec![5, 1, 0, 3, 0, 1]);
    }

    #[tokio::test]
    async fn vnc_password_auth() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move { fake_server(l.accept().await.unwrap().0, 8, Some("s3cret")).await });
        let view = LiveView::connect_tcp(&addr, Some("s3cret")).await.unwrap();
        wait_gen(&view).await;
        view.send(Input::Pointer { x: 0, y: 0, buttons: 0 });
        server.await.unwrap();

        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap().to_string();
        tokio::spawn(async move { fake_server(l.accept().await.unwrap().0, 8, Some("right")).await });
        assert!(LiveView::connect_tcp(&addr, Some("wrong")).await.unwrap_err().contains("denied"));
    }

    #[tokio::test]
    async fn over_websocket_like_websockify() {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (tcp, _) = l.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_hdr_async(tcp, |req: &Request, mut resp: Response| {
                assert!(req.headers().get("sec-websocket-protocol").is_some_and(|v| v == "binary"));
                resp.headers_mut().insert("Sec-WebSocket-Protocol", "binary".parse().unwrap());
                Ok(resp)
            })
            .await
            .unwrap();
            // Bridge the socket to a byte stream, then run the same fake server.
            let (a, b) = tokio::io::duplex(1 << 16);
            let (mut brd, mut bwr) = tokio::io::split(b);
            let (mut sink, mut stream) = ws.split();
            tokio::spawn(async move {
                while let Some(Ok(WsMessage::Binary(m))) = stream.next().await {
                    // Deliver in 3-byte pieces to prove the client reassembles.
                    for c in m.chunks(3) {
                        bwr.write_all(c).await.unwrap();
                    }
                }
            });
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                loop {
                    let n = brd.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    for c in buf[..n].chunks(5) {
                        if sink.send(WsMessage::Binary(c.to_vec().into())).await.is_err() {
                            return;
                        }
                    }
                }
            });
            fake_server(a, 8, None).await
        });
        let view = LiveView::connect_ws(&format!("ws://127.0.0.1:{port}/websockify"), None).await.unwrap();
        wait_gen(&view).await;
        view.send(Input::Pointer { x: 2, y: 1, buttons: 4 });
        assert_eq!(server.await.unwrap(), vec![5, 4, 0, 2, 0, 1]);
    }

    #[test]
    fn keysyms() {
        assert_eq!(char_keysym('a'), 0x61);
        assert_eq!(char_keysym('é'), 0xe9);
        assert_eq!(char_keysym('✓'), 0x0100_2713);
        assert_eq!(named_keysym("Enter"), Some(0xff0d));
        assert_eq!(named_keysym("F5"), Some(0xffc2));
    }
}
