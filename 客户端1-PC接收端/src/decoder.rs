//! H.264 解码器：把网络收到的 Annex B NAL 喂给 FFmpeg，解出 BGRA 帧
//!
//! 采用 FFmpeg 子进程 + 管道（stdin 写 NAL，stdout 读 BGRA），
//! 避免链接 FFmpeg C API 的复杂构建；解码线程独立，帧回调只入队。
//!
//! 关键约定：
//! - 视频帧布局由协商决定。ALVR H.264 编码为「两眼合成一张帧」：
//!   SBS（左右并排，宽=2×单眼宽）或 TB（上下）。由 layout 参数传给渲染器。
//! - 解码分辨率 = view_resolution × 2（SBS）或 × (1,2)（TB）。

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

#[derive(Clone)]
pub struct DecodeConfig {
    pub ffmpeg: String,
    pub width: u32,   // 单眼宽
    pub height: u32,  // 单眼高
    pub layout: u32,  // 0=SBS 左右, 1=TB 上下
    pub fps: f32,
}

impl Default for DecodeConfig {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg.exe".into(),
            width: 1536,
            height: 1536,
            layout: 0,
            fps: 70.0,
        }
    }
}

impl DecodeConfig {
    fn frame_size(&self) -> (u32, u32) {
        match self.layout {
            0 => (self.width * 2, self.height), // SBS
            _ => (self.width, self.height * 2), // TB
        }
    }
    fn ffmpeg_args(&self) -> Vec<String> {
        let (w, h) = self.frame_size();
        vec![
            "-hide_banner".into(), "-loglevel".into(), "error".into(),
            "-fflags".into(), "nobuffer".into(),
            "-f".into(), "h264".into(),
            "-framerate".into(), format!("{}", self.fps),
            "-i".into(), "pipe:0".into(),
            "-f".into(), "rawvideo".into(),
            "-pix_fmt".into(), "bgra".into(),
            "-vf".into(), format!("scale={}:{}:flags=fast_bilinear", w, h),
            "-vsync".into(), "0".into(),
            "pipe:1".into(),
        ]
    }
}

/// 解码器句柄：push_nal 入队；内部线程写 FFmpeg 并读出 BGRA 帧交给回调
pub struct Decoder {
    nal_tx: std::sync::mpsc::Sender<Vec<u8>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Decoder {
    pub fn start<F>(cfg: DecodeConfig, on_bgra: F) -> Result<Self, String>
    where
        F: FnMut(&[u8], u32, u32) + Send + 'static,
    {
        // [HVR fix] ffmpeg stderr 绝不能用 Stdio::piped() 却不读取：
        // ffmpeg 一报错（SPS/PPS 不匹配、分辨率不符等）就往 stderr 写，
        // 管道缓冲 (~64KB) 写满后 ffmpeg 阻塞，停止向 stdout 输出 BGRA，
        // 于是渲染器永远等不到首帧。改为重定向到文件，永不阻塞。
        let err_file = std::fs::File::create("ffmpeg_stderr.log")
            .ok();
        let mut cmd = Command::new(&cfg.ffmpeg);
        cmd.args(cfg.ffmpeg_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        if err_file.is_some() {
            cmd.stderr(Stdio::from(err_file.unwrap()));
        } else {
            cmd.stderr(Stdio::null());
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动 FFmpeg 失败（请把 ffmpeg.exe 放到同目录或 PATH）: {e}"))?;

        let stdin = child.stdin.take().ok_or("FFmpeg stdin 不可用")?;
        let stdout = child.stdout.take().ok_or("FFmpeg stdout 不可用")?;
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let cfg2 = cfg.clone();
        let stop2 = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            decoder_thread(cfg2, stdin, stdout, child, rx, stop2, on_bgra);
        });

        Ok(Self {
            nal_tx: tx,
            stop,
            thread: Some(handle),
        })
    }

    /// 提交一个 Annex B NAL；返回 false 表示队列饱和（触发请求 IDR）
    pub fn push_nal(&self, nal: &[u8]) -> bool {
        if self.nal_tx.send(nal.to_vec()).is_err() {
            return false;
        }
        true
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

fn decoder_thread<F>(
    cfg: DecodeConfig,
    mut stdin: ChildStdin,
    mut stdout: ChildStdout,
    mut child: Child,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    stop: Arc<AtomicBool>,
    mut on_bgra: F,
) where
    F: FnMut(&[u8], u32, u32),
{
    let (w, h) = cfg.frame_size();
    let frame_bytes = (w as usize) * (h as usize) * 4;

    // 写线程：NAL → FFmpeg stdin
    let stop_w = Arc::clone(&stop);
    let writer = thread::spawn(move || {
        // [HVR fix] 服务端未下发 SPS/PPS（DecoderConfig 通道不通）时，注入默认
        // H.264 SPS/PPS（3072x1536 = SBS 双目 1536x1536），让 ffmpeg 能初始化解码器。
        // 取自 libx264 对 3072x1536 的输出，profile/level 与服务端 SW 编码兼容。
        const DEFAULT_SPSPPS: &[u8] = &[
            0x00,0x00,0x00,0x01,0x67,0x42,0xc0,0x32,0xda,0x00,0xc0,0x03,0x06,0xc0,0x44,0x00,
            0x00,0x03,0x00,0x04,0x00,0x00,0x03,0x00,0xc8,0x3c,0x60,0xca,0x80,
            0x00,0x00,0x00,0x01,0x68,0xce,0x0f,0xc8,
        ];
        if stdin.write_all(DEFAULT_SPSPPS).is_ok() { let _ = stdin.flush(); }
        let mut nal_count: u64 = 0;
        let mut nal_bytes: u64 = 0;
        while !stop_w.load(Ordering::Relaxed) {
            match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(nal) => {
                    nal_count += 1;
                    nal_bytes += nal.len() as u64;
                    if nal_count == 1 || nal_count % 300 == 0 {
                        eprintln!("[decoder] 已送 ffmpeg {nal_count} 个NAL / {nal_bytes} 字节 (最新{}B, head={:02X?})",
                            nal.len(), &nal[..nal.len().min(8)]);
                    }
                    if stdin.write_all(&nal).is_err() {
                        eprintln!("[decoder] ffmpeg stdin 写入失败，解码器退出");
                        break;
                    }
                    let _ = stdin.flush();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    // 读线程（本线程）：FFmpeg stdout → BGRA 帧 → 回调
    // [HVR fix] 单帧 BGRA 可达 ~19MB，100ms 内读不完；超时会丢弃已读部分导致永远组不出帧。
    // 改为：单次 read 阻塞读取（不设短超时），只在 ffmpeg 退出(Eof)时结束。
    let mut frame = vec![0u8; frame_bytes];
    let mut frame_count: u64 = 0;
    eprintln!("[decoder] 读帧线程就绪, 单帧={}x{}x4={}B", w, h, frame_bytes);
    while !stop.load(Ordering::Relaxed) {
        match read_exact_blocking(&mut stdout, &mut frame) {
            Ok(()) => {
                frame_count += 1;
                if frame_count == 1 || frame_count % 100 == 0 {
                    eprintln!("[decoder] 已解出 {frame_count} 帧 BGRA");
                }
                on_bgra(&frame, w, h);
            }
            Err(ReadErr::Eof) => {
                eprintln!("[decoder] ffmpeg stdout EOF, 共解出 {frame_count} 帧");
                break;
            }
        }
    }
    let _ = writer.join();
    let _ = child.kill();
    let _ = child.wait();
}

enum ReadErr { Eof }

/// 阻塞读满一帧；ffmpeg 输出流在进程存活时不会半途返回 0，故无需超时。
fn read_exact_blocking(r: &mut impl Read, buf: &mut [u8]) -> Result<(), ReadErr> {
    let mut off = 0;
    while off < buf.len() {
        match r.read(&mut buf[off..]) {
            Ok(0) => return Err(ReadErr::Eof),
            Ok(n) => off += n,
            Err(_) => return Err(ReadErr::Eof),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_size_sbs_and_tb() {
        let mut c = DecodeConfig::default();
        c.width = 1536; c.height = 1536;
        assert_eq!(c.frame_size(), (3072, 1536)); // SBS
        c.layout = 1;
        assert_eq!(c.frame_size(), (1536, 3072)); // TB
    }

    #[test]
    fn ffmpeg_args_contain_rawvideo_bgra() {
        let c = DecodeConfig::default();
        let a = c.ffmpeg_args();
        assert!(a.iter().any(|x| x == "rawvideo"));
        assert!(a.iter().any(|x| x == "bgra"));
        assert!(a.iter().any(|x| x == "h264"));
        assert!(a.iter().any(|x| x.contains("scale=3072:1536")));
    }
}
