use eframe::egui;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Non-blocking file dialog runner so the UI keeps redrawing if the window is resized while a dialog is open.
pub struct PendingFileDialog {
    rx: std::sync::mpsc::Receiver<Option<PathBuf>>,
}

impl PendingFileDialog {
    pub fn spawn(
        ctx: &egui::Context,
        f: impl FnOnce() -> Option<PathBuf> + Send + 'static,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx_clone = ctx.clone();
        std::thread::spawn(move || {
            let res = f();
            let _ = tx.send(res);
            ctx_clone.request_repaint();
        });
        Self { rx }
    }

    pub fn poll(&self) -> Option<Option<PathBuf>> {
        self.rx.try_recv().ok()
    }
}

pub struct UndoHistory<T> {
    undo_stack: Vec<T>,
    redo_stack: Vec<T>,
    drag_start: Option<T>,
}

impl<T> Default for UndoHistory<T> {
    fn default() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            drag_start: None,
        }
    }
}

impl<T: Clone + PartialEq> UndoHistory<T> {
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.drag_start = None;
    }

    pub fn record_frame(&mut self, before: &T, after: &T, pointer_down: bool) {
        if before != after {
            if pointer_down {
                if self.drag_start.is_none() {
                    self.drag_start = Some(before.clone());
                }
            } else {
                let initial = self.drag_start.take().unwrap_or_else(|| before.clone());
                if &initial != after {
                    self.push_undo(initial);
                }
            }
        } else if !pointer_down {
            if let Some(initial) = self.drag_start.take() {
                if &initial != after {
                    self.push_undo(initial);
                }
            }
        }
    }

    fn push_undo(&mut self, state: T) {
        if self.undo_stack.last() != Some(&state) {
            self.undo_stack.push(state);
            if self.undo_stack.len() > 60 {
                self.undo_stack.remove(0);
            }
        }
        self.redo_stack.clear();
    }

    pub fn undo(&mut self, current: &T) -> Option<T> {
        self.drag_start = None;
        let prev = self.undo_stack.pop()?;
        self.redo_stack.push(current.clone());
        Some(prev)
    }

    pub fn redo(&mut self, current: &T) -> Option<T> {
        self.drag_start = None;
        let next = self.redo_stack.pop()?;
        self.undo_stack.push(current.clone());
        Some(next)
    }
}

#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    pub duration_secs: f64,
    pub width: u32,
    pub height: u32,
    pub codec: String,
    pub bitrate_kbps: u32,
    pub file_size: u64,
}

pub fn check_ffmpeg_available() -> bool {
    hidden_command("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn probe_media(path: &Path) -> Result<MediaInfo, String> {
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    let output = hidden_command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration,bit_rate:stream=width,height,codec_name,codec_type",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("Не удалось запустить ffprobe: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Ошибка чтения файла: {}", stderr.trim()));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut info = MediaInfo {
        file_size,
        ..Default::default()
    };

    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim();
            match k.trim() {
                "duration" => {
                    if let Ok(d) = v.parse::<f64>() {
                        if d > info.duration_secs {
                            info.duration_secs = d;
                        }
                    }
                }
                "bit_rate" => {
                    if let Ok(br) = v.parse::<u64>() {
                        info.bitrate_kbps = (br / 1000) as u32;
                    }
                }
                "width" => {
                    if let Ok(w) = v.parse::<u32>() {
                        if w > 0 {
                            info.width = w;
                        }
                    }
                }
                "height" => {
                    if let Ok(h) = v.parse::<u32>() {
                        if h > 0 {
                            info.height = h;
                        }
                    }
                }
                "codec_name" => {
                    if info.codec.is_empty() && !v.is_empty() {
                        info.codec = v.to_uppercase();
                    }
                }
                _ => {}
            }
        }
    }

    if info.duration_secs <= 0.0 {
        info.duration_secs = 1.0;
    }

    Ok(info)
}

/// Extracts a single preview frame from a video at `time_secs`, scaled to fit 960px width.
pub fn push_video_transform_filters(
    vf_parts: &mut Vec<String>,
    rotation_deg: u32,
    flip_h: bool,
    flip_v: bool,
) {
    match rotation_deg % 360 {
        90 => vf_parts.push("transpose=1".to_string()),
        180 => vf_parts.push("transpose=1,transpose=1".to_string()),
        270 => vf_parts.push("transpose=2".to_string()),
        _ => {}
    }
    if flip_h {
        vf_parts.push("hflip".to_string());
    }
    if flip_v {
        vf_parts.push("vflip".to_string());
    }
}

pub fn extract_video_frame(
    path: &Path,
    time_secs: f64,
    rotation_deg: u32,
    flip_h: bool,
    flip_v: bool,
) -> Result<egui::ColorImage, String> {
    let mut vf_parts = vec!["scale='min(960,iw)':-2".to_string()];
    push_video_transform_filters(&mut vf_parts, rotation_deg, flip_h, flip_v);
    let vf = vf_parts.join(",");

    let output = hidden_command("ffmpeg")
        .args([
            "-v",
            "error",
            "-ss",
            &format!("{:.3}", time_secs.max(0.0)),
            "-i",
        ])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &vf,
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "-",
        ])
        .output()
        .map_err(|e| format!("Ошибка ffmpeg: {}", e))?;

    if !output.status.success() || output.stdout.is_empty() {
        return Err("Не удалось получить кадр видео".to_string());
    }

    let img = image::load_from_memory(&output.stdout)
        .map_err(|e| format!("Ошибка декодирования кадра: {}", e))?;
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// Decodes audio into mono 8kHz PCM and downsamples into `bars` peak amplitudes in 0.0..=1.0.
pub fn extract_waveform(path: &Path, bars: usize) -> Result<Vec<f32>, String> {
    let output = hidden_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-ac", "1",
            "-ar", "4000",
            "-f", "s16le",
            "-",
        ])
        .output()
        .map_err(|e| format!("Ошибка ffmpeg: {}", e))?;

    if !output.status.success() || output.stdout.len() < 4 {
        return Err("Не удалось построить звуковую волну".to_string());
    }

    let raw = &output.stdout;
    let total_samples = raw.len() / 2;
    if total_samples == 0 || bars == 0 {
        return Ok(vec![0.1; bars]);
    }

    let chunk_size = (total_samples / bars).max(1);
    let mut peaks = Vec::with_capacity(bars);

    for i in 0..bars {
        let start = i * chunk_size;
        let end = ((i + 1) * chunk_size).min(total_samples);
        let mut max_amp: f32 = 0.0;
        for s in start..end {
            let idx = s * 2;
            let sample = i16::from_le_bytes([raw[idx], raw[idx + 1]]);
            let amp = (sample as f32 / i16::MAX as f32).abs();
            if amp > max_amp {
                max_amp = amp;
            }
        }
        peaks.push(max_amp.clamp(0.03, 1.0));
    }

    // Normalize softly so quiet tracks still look readable
    let global_max = peaks.iter().copied().fold(0.0_f32, f32::max);
    if global_max > 0.05 {
        for p in &mut peaks {
            *p = (*p / global_max).clamp(0.04, 1.0);
        }
    }

    Ok(peaks)
}

pub fn reveal_in_explorer(path: &Path) {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer")
            .arg("/select,")
            .arg(path)
            .spawn();
    }
}

/// Safely moves a file to the Windows Recycle Bin using Shell API.
#[cfg(windows)]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    struct ShFileOpStructW {
        hwnd: isize,
        w_func: u32,
        p_from: *const u16,
        p_to: *const u16,
        f_flags: u16,
        f_any_operations_aborted: i32,
        h_name_mappings: isize,
        lpsz_progress_title: *const u16,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(lp_file_op: *mut ShFileOpStructW) -> i32;
    }

    const FO_DELETE: u32 = 3;
    const FOF_SILENT: u16 = 0x0004;
    const FOF_NOCONFIRMATION: u16 = 0x0010;
    const FOF_ALLOWUNDO: u16 = 0x0040;
    const FOF_NOERRORUI: u16 = 0x0400;

    if !path.exists() {
        return Err("Файл не найден".to_string());
    }

    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let path_str = abs.to_string_lossy();
    let clean = path_str.strip_prefix(r"\\?\").unwrap_or(&path_str);

    let mut wide: Vec<u16> = OsStr::new(clean).encode_wide().collect();
    wide.push(0);
    wide.push(0);

    let mut op = ShFileOpStructW {
        hwnd: 0,
        w_func: FO_DELETE,
        p_from: wide.as_ptr(),
        p_to: std::ptr::null(),
        f_flags: FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI,
        f_any_operations_aborted: 0,
        h_name_mappings: 0,
        lpsz_progress_title: std::ptr::null(),
    };

    let code = unsafe { SHFileOperationW(&mut op) };
    if code == 0 && op.f_any_operations_aborted == 0 {
        Ok(())
    } else {
        Err(format!("Не удалось переместить файл в корзину (код {})", code))
    }
}

#[cfg(not(windows))]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| format!("Не удалось удалить файл: {}", e))
}

/// Streams stereo 44.1kHz PCM from `ffmpeg` into Windows `winmm` waveOut with lock-free live volume control.
#[cfg(windows)]
pub fn spawn_live_audio_stream(
    path: PathBuf,
    start_time: f64,
    duration: f64,
    volume_bits: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
) -> Option<Arc<Mutex<Option<Child>>>> {
    #[repr(C, packed)]
    struct WaveFormatEx {
        w_format_tag: u16,
        n_channels: u16,
        n_samples_per_sec: u32,
        n_avg_bytes_per_sec: u32,
        n_block_align: u16,
        w_bits_per_sample: u16,
        cb_size: u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WaveHdr {
        lp_data: *mut u8,
        dw_buffer_length: u32,
        dw_bytes_recorded: u32,
        dw_user: usize,
        dw_flags: u32,
        dw_loops: u32,
        lp_next: *mut WaveHdr,
        reserved: usize,
    }

    #[link(name = "winmm")]
    extern "system" {
        fn waveOutOpen(
            phwo: *mut isize,
            u_device_id: u32,
            pwfx: *const WaveFormatEx,
            dw_callback: usize,
            dw_instance: usize,
            fdw_open: u32,
        ) -> u32;
        fn waveOutPrepareHeader(hwo: isize, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        fn waveOutWrite(hwo: isize, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        fn waveOutUnprepareHeader(hwo: isize, pwh: *mut WaveHdr, cbwh: u32) -> u32;
        fn waveOutReset(hwo: isize) -> u32;
        fn waveOutClose(hwo: isize) -> u32;
    }

    const WAVE_MAPPER: u32 = u32::MAX;
    const WAVE_FORMAT_PCM: u16 = 1;
    const WHDR_DONE: u32 = 0x0000_0001;
    const NUM_BUFS: usize = 4;
    const BUF_BYTES: usize = 7056; // 40ms of 44100 Hz stereo 16-bit PCM

    let mut cmd = hidden_command("ffmpeg");
    cmd.args([
        "-v",
        "error",
        "-ss",
        &format!("{:.3}", start_time.max(0.0)),
        "-i",
    ])
    .arg(&path)
    .args([
        "-t",
        &format!("{:.3}", duration.max(0.1)),
        "-vn",
        "-ac",
        "2",
        "-ar",
        "44100",
        "-f",
        "s16le",
        "-",
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::null());

    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let child_arc = Arc::new(Mutex::new(Some(child)));
    let child_thread = Arc::clone(&child_arc);

    std::thread::spawn(move || {
        let wfx = WaveFormatEx {
            w_format_tag: WAVE_FORMAT_PCM,
            n_channels: 2,
            n_samples_per_sec: 44100,
            n_avg_bytes_per_sec: 176400,
            n_block_align: 4,
            w_bits_per_sample: 16,
            cb_size: 0,
        };

        let mut hwo: isize = 0;
        let open_res = unsafe { waveOutOpen(&mut hwo, WAVE_MAPPER, &wfx, 0, 0, 0) };
        if open_res != 0 || hwo == 0 {
            if let Ok(mut g) = child_thread.lock() {
                if let Some(mut c) = g.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
            }
            return;
        }

        let mut buffers: Vec<Box<[u8]>> = (0..NUM_BUFS)
            .map(|_| vec![0u8; BUF_BYTES].into_boxed_slice())
            .collect();
        let zero_hdr = WaveHdr {
            lp_data: std::ptr::null_mut(),
            dw_buffer_length: 0,
            dw_bytes_recorded: 0,
            dw_user: 0,
            dw_flags: 0,
            dw_loops: 0,
            lp_next: std::ptr::null_mut(),
            reserved: 0,
        };
        let mut headers: Box<[WaveHdr]> = vec![zero_hdr; NUM_BUFS].into_boxed_slice();
        let mut prepared = [false; NUM_BUFS];
        let hdr_size = std::mem::size_of::<WaveHdr>() as u32;

        let mut raw = vec![0u8; BUF_BYTES];
        let mut slot: usize = 0;
        let mut reached_eof = false;

        loop {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            // Read up to BUF_BYTES from ffmpeg stdout
            let mut filled = 0;
            while filled < BUF_BYTES {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                match stdout.read(&mut raw[filled..]) {
                    Ok(0) => {
                        reached_eof = true;
                        break;
                    }
                    Ok(n) => filled += n,
                    Err(_) => {
                        reached_eof = true;
                        break;
                    }
                }
            }

            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            let aligned_len = (filled / 4) * 4;
            if aligned_len == 0 {
                break;
            }

            // Wait until the next ring slot finishes playback
            loop {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                if !prepared[slot] {
                    break;
                }
                let flags = unsafe { std::ptr::read_volatile(&headers[slot].dw_flags) };
                if (flags & WHDR_DONE) != 0 {
                    unsafe {
                        waveOutUnprepareHeader(hwo, &mut headers[slot], hdr_size);
                    }
                    prepared[slot] = false;
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }

            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            let gain = if muted.load(Ordering::Relaxed) {
                0.0_f32
            } else {
                (f32::from_bits(volume_bits.load(Ordering::Relaxed)) / 100.0).clamp(0.0, 3.0)
            };

            let dst = &mut buffers[slot];
            for i in (0..aligned_len).step_by(2) {
                let s = i16::from_le_bytes([raw[i], raw[i + 1]]) as f32;
                let scaled = (s * gain).round().clamp(-32768.0, 32767.0) as i16;
                let bytes = scaled.to_le_bytes();
                dst[i] = bytes[0];
                dst[i + 1] = bytes[1];
            }

            headers[slot] = WaveHdr {
                lp_data: buffers[slot].as_mut_ptr(),
                dw_buffer_length: aligned_len as u32,
                dw_bytes_recorded: 0,
                dw_user: 0,
                dw_flags: 0,
                dw_loops: 0,
                lp_next: std::ptr::null_mut(),
                reserved: 0,
            };

            let prep = unsafe { waveOutPrepareHeader(hwo, &mut headers[slot], hdr_size) };
            if prep == 0 {
                prepared[slot] = true;
                let _ = unsafe { waveOutWrite(hwo, &mut headers[slot], hdr_size) };
            }

            slot = (slot + 1) % NUM_BUFS;
            if reached_eof {
                break;
            }
        }

        // Wait for queued audio to finish if stream ended naturally
        if reached_eof && !stop_flag.load(Ordering::Relaxed) {
            for i in 0..NUM_BUFS {
                while prepared[i] && !stop_flag.load(Ordering::Relaxed) {
                    let flags = unsafe { std::ptr::read_volatile(&headers[i].dw_flags) };
                    if (flags & WHDR_DONE) != 0 {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(8));
                }
            }
        }

        unsafe {
            waveOutReset(hwo);
            for i in 0..NUM_BUFS {
                if prepared[i] {
                    waveOutUnprepareHeader(hwo, &mut headers[i], hdr_size);
                }
            }
            waveOutClose(hwo);
        }

        if let Ok(mut g) = child_thread.lock() {
            if let Some(mut c) = g.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    });

    Some(child_arc)
}

#[cfg(not(windows))]
pub fn spawn_live_audio_stream(
    _path: PathBuf,
    _start_time: f64,
    _duration: f64,
    _volume_bits: Arc<AtomicU32>,
    _muted: Arc<AtomicBool>,
    _stop_flag: Arc<AtomicBool>,
) -> Option<Arc<Mutex<Option<Child>>>> {
    None
}
