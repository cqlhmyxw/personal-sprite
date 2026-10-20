// ============ 个人精灵 · Rust 后端 ============
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

// ---------- 配置 ----------
const APP_DIR_NAME: &str = "PersonalSprite";
const TAPI_KEY: &str = match option_env!("TAPI_KEY") {
    Some(v) => v,
    None => "xiaokong-translate-2026",
};
const TAPI_BASE: &str = "https://tapi.fast-api-hub.top/api";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) PersonalSprite/2.0";

fn data_dir(app: &AppHandle) -> PathBuf {
    let base = app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from("."));
    let dir = base.join(APP_DIR_NAME);
    fs::create_dir_all(&dir).ok();
    dir
}

fn notes_dir(app: &AppHandle) -> PathBuf {
    let d = data_dir(app).join("notes");
    fs::create_dir_all(&d).ok();
    d
}

// ============ 窗口控制 ============
#[tauri::command]
fn minimize_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        w.minimize().ok();
    }
}

#[tauri::command]
fn close_window(app: AppHandle) {
    // V2.1: 关闭主窗口=隐藏到后台，后台翻译线程继续工作（常驻）
    if let Some(w) = app.get_webview_window("main") {
        w.hide().ok();
    }
}

// 真正退出（悬浮窗上提供退出按钮）
#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

// ============ 翻译 ============
#[derive(Serialize, Deserialize)]
struct TransResp {
    translated_text: String,
}

#[tauri::command]
fn translate(text: String, target: String, source: String) -> Result<String, String> {
    let src = if source.is_empty() || source == "auto" { "auto" } else { &source };
    let resp = ureq::post(&format!("{TAPI_BASE}/translate"))
        .set("X-API-Key", TAPI_KEY)
        .set("User-Agent", UA)
        .send_form(&[
            ("text", text.as_str()),
            ("target", target.as_str()),
            ("source", src),
        ])
        .map_err(|e| format!("网络错误: {e}"))?;
    let body_str = resp.into_string().map_err(|e| format!("读取失败: {e}"))?;
    let body: TransResp = serde_json::from_str(&body_str).map_err(|e| format!("解析失败: {e}"))?;
    if body.translated_text.is_empty() {
        return Err("翻译结果为空".into());
    }
    Ok(body.translated_text)
}

// ============ 剪贴板 & 朗读 ============
#[tauri::command]
fn copy_text(app: AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

#[tauri::command]
fn speak_text(text: String) -> Result<(), String> {
    // V1: 打开系统默认浏览器朗读占位；V3 接自家 TTS 网关
    let _ = text;
    Ok(())
}

// ============ V2.1 沉浸式翻译：剪贴板监听 ============
// 前端每 800ms 轮询剪贴板，检测到复制了新文本就自动翻译
#[tauri::command]
fn clipboard_poll(app: AppHandle, last_text: String) -> Result<String, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let current = app.clipboard().read_text().map_err(|e| e.to_string())?;
    // 剪贴板内容没变 → 返回空（前端不翻译）
    if current.is_empty() || current == last_text {
        return Ok(String::new());
    }
    Ok(current)
}

// ============ V2.2 屏幕框选 OCR 翻译 ============
// 前端框选区域 → 传坐标 → Rust 截屏（后续接 OCR）
#[tauri::command]
fn screen_capture_region(x: i32, y: i32, w: i32, h: i32) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "windows")]
    {
        // Windows: 用 PowerShell + System.Drawing 截屏（无额外依赖）
        let ps = format!(
            "Add-Type -AssemblyName System.Drawing; \
             Add-Type -AssemblyName System.Windows.Forms; \
             $b = New-Object System.Drawing.Bitmap({w},{h}); \
             $g = [System.Drawing.Graphics]::FromImage($b); \
             $g.CopyFromScreen({x},{y},0,0,({w},{h})); \
             $ms = New-Object System.IO.MemoryStream; \
             $b.Save($ms,[System.Drawing.Imaging.ImageFormat]::Png); \
             [Convert]::ToBase64String($ms.ToArray())",
            x = x, y = y, w = w, h = h
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .output()
            .map_err(|e| format!("截屏失败: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        let b64 = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let bytes = base64_decode(&b64)?;
        Ok(bytes)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (x, y, w, h);
        Err("非 Windows 平台暂不支持截屏".into())
    }
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("解码失败: {e}"))
}

// ============ 笔记数据模型 ============
#[derive(Serialize, Deserialize, Clone)]
struct Notebook {
    id: String,
    name: String,
    pages: Vec<Page>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct Page {
    id: String,
    title: String,
    content: String,
}

fn nb_path(app: &AppHandle) -> PathBuf {
    notes_dir(app).join("notebooks.json")
}

fn load_nbs(app: &AppHandle) -> Vec<Notebook> {
    let p = nb_path(app);
    if p.exists() {
        fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        vec![]
    }
}

fn save_nbs(app: &AppHandle, nbs: &[Notebook]) {
    let p = nb_path(app);
    if let Ok(s) = serde_json::to_string_pretty(nbs) {
        fs::write(p, s).ok();
    }
}

fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!(
        "{:x}{:x}",
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
        std::process::id()
    )
}

// ---------- 笔记命令 ----------
#[tauri::command]
fn list_notebooks(app: AppHandle) -> Vec<Notebook> {
    load_nbs(&app)
}

#[tauri::command]
fn create_notebook(app: AppHandle, name: String) -> Result<Notebook, String> {
    let mut nbs = load_nbs(&app);
    let nb = Notebook { id: new_id(), name, pages: vec![] };
    nbs.push(nb.clone());
    save_nbs(&app, &nbs);
    Ok(nb)
}

#[tauri::command]
fn list_pages(app: AppHandle, notebook_id: String) -> Result<Vec<Page>, String> {
    let nbs = load_nbs(&app);
    let nb = nbs.iter().find(|n| n.id == notebook_id).ok_or("笔记本不存在")?;
    Ok(nb.pages.clone())
}

#[tauri::command]
fn create_page(app: AppHandle, notebook_id: String) -> Result<Page, String> {
    let mut nbs = load_nbs(&app);
    let nb = nbs.iter_mut().find(|n| n.id == notebook_id).ok_or("笔记本不存在")?;
    let page = Page { id: new_id(), title: "新页面".into(), content: String::new() };
    nb.pages.push(page.clone());
    save_nbs(&app, &nbs);
    Ok(page)
}

#[tauri::command]
fn load_page(app: AppHandle, page_id: String) -> Result<Page, String> {
    let nbs = load_nbs(&app);
    for nb in &nbs {
        if let Some(p) = nb.pages.iter().find(|p| p.id == page_id) {
            return Ok(p.clone());
        }
    }
    Err("页面不存在".into())
}

#[tauri::command]
fn save_page(app: AppHandle, page_id: String, title: String, content: String) -> Result<(), String> {
    let mut nbs = load_nbs(&app);
    for nb in &mut nbs {
        if let Some(p) = nb.pages.iter_mut().find(|p| p.id == page_id) {
            p.title = title;
            p.content = content;
            save_nbs(&app, &nbs);
            return Ok(());
        }
    }
    Err("页面不存在".into())
}

#[tauri::command]
fn export_page(app: AppHandle, page_id: String) -> Result<String, String> {
    let nbs = load_nbs(&app);
    for nb in &nbs {
        if let Some(p) = nb.pages.iter().find(|p| p.id == page_id) {
            let mut md = format!("# {}\n\n", p.title);
            // 简单 HTML → Markdown（V3 用 pandoc 做全量）
            let html = &p.content;
            md.push_str(&html.replace("<br>", "\n").replace("<br/>", "\n"));
            let out = data_dir(&app).join("导出").join(format!("{}.md", sanitize(&p.title)));
            fs::create_dir_all(out.parent().unwrap()).ok();
            fs::write(&out, md).map_err(|e| e.to_string())?;
            return Ok(out.to_string_lossy().into_owned());
        }
    }
    Err("页面不存在".into())
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c }).collect()
}

#[tauri::command]
fn attach_file(app: AppHandle, page_id: String) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    let file = app.dialog().file().blocking_pick_file();
    let path = file.ok_or("未选择文件")?;
    let src = path.to_string();
    let attach_dir = data_dir(&app).join("附件").join(&page_id);
    fs::create_dir_all(&attach_dir).ok();
    let fname = Path::new(&src).file_name().ok_or("无效文件名")?.to_string_lossy().into_owned();
    let dest = attach_dir.join(fname);
    fs::copy(&src, &dest).map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().into_owned())
}

#[tauri::command]
fn voice_record() -> Result<String, String> {
    // V3 接入：Windows 录音 → 自家 Whisper STT 网关
    Err("语音功能开发中（V3 上线）".into())
}

// ============ 文件转换 ============
#[tauri::command]
fn pick_file(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    let file = app.dialog().file().blocking_pick_file();
    file.map(|f| f.to_string()).ok_or_else(|| "未选择文件".into())
}

#[derive(Serialize)]
struct TargetInfo {
    ext: String,
    label: String,
}

fn ext_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

#[tauri::command]
fn convert_targets(file_path: String) -> Vec<TargetInfo> {
    let ext = ext_of(&file_path);
    let map: &[(&str, &[&str])] = &[
        ("pdf", &["docx", "doc", "txt", "md", "html", "png", "jpg", "epub"]),
        ("docx", &["pdf", "txt", "md", "html", "epub"]),
        ("doc", &["pdf", "docx", "txt", "md"]),
        ("txt", &["pdf", "docx", "md", "html"]),
        ("md", &["pdf", "docx", "html", "txt"]),
        ("html", &["pdf", "docx", "txt", "md"]),
        ("epub", &["pdf", "docx", "txt", "md", "html"]),
        ("png", &["jpg", "webp", "bmp", "gif", "pdf"]),
        ("jpg", &["png", "webp", "bmp", "gif", "pdf"]),
        ("jpeg", &["png", "webp", "bmp", "gif", "pdf"]),
        ("webp", &["png", "jpg", "bmp", "gif"]),
        ("bmp", &["png", "jpg", "webp", "gif"]),
        ("gif", &["png", "jpg", "webp", "bmp"]),
        ("mp4", &["mkv", "avi", "mov", "webm", "gif", "mp3", "wav"]),
        ("mkv", &["mp4", "avi", "mov", "webm", "mp3", "wav"]),
        ("mov", &["mp4", "mkv", "avi", "webm", "mp3"]),
        ("avi", &["mp4", "mkv", "mov", "webm", "mp3"]),
        ("webm", &["mp4", "mkv", "avi", "mov", "mp3"]),
        ("mp3", &["wav", "flac", "ogg", "aac", "m4a"]),
        ("wav", &["mp3", "flac", "ogg", "aac", "m4a"]),
        ("flac", &["mp3", "wav", "ogg", "aac"]),
        ("ogg", &["mp3", "wav", "flac", "aac"]),
    ];
    map.iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, targets)| {
            targets.iter().map(|t| TargetInfo { ext: t.to_string(), label: format!("{} → .{}", ext.to_uppercase(), t) }).collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
fn convert_file(app: AppHandle, file_path: String, target_ext: String) -> Result<String, String> {
    let src_ext = ext_of(&file_path);
    let out_dir = data_dir(&app).join("转换结果");
    fs::create_dir_all(&out_dir).ok();
    let base = Path::new(&file_path).file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let out_path = out_dir.join(format!("{}.{}", base, target_ext));
    let out_str = out_path.to_string_lossy().into_owned();

    // 文档类：PDF 走云端 API；txt/md/html 本地直转；其他报错
    let doc_exts = ["pdf", "docx", "doc", "txt", "md", "html", "epub"];
    let img_exts = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];
    let media_exts = ["mp4", "mkv", "mov", "avi", "webm", "mp3", "wav", "flac", "ogg", "aac", "m4a"];

    if src_ext == "pdf" && target_ext == "docx" {
        // PDF → Word：走云端转换 API（本地无引擎也不怕）
        cloud_pdf_to_docx(&file_path, &out_str).map_err(|e| format!("PDF转Word失败: {e}"))?;
    } else if doc_exts.contains(&src_ext.as_str()) && doc_exts.contains(&target_ext.as_str()) {
        run_pandoc(&file_path, &out_str).map_err(|e| format!("文档转换失败: {e}"))?;
    } else if img_exts.contains(&src_ext.as_str()) && img_exts.contains(&target_ext.as_str()) {
        run_pillow(&file_path, &out_str).map_err(|e| format!("图片转换失败: {e}"))?;
    } else if media_exts.contains(&src_ext.as_str()) || media_exts.contains(&target_ext.as_str()) {
        run_ffmpeg(&file_path, &out_str).map_err(|e| format!("音视频转换失败: {e}"))?;
    } else {
        return Err("暂不支持该转换".into());
    }
    Ok(out_str)
}

// PDF → Word：上传到云端 translate-file API，返回 docx 文件
fn cloud_pdf_to_docx(src: &str, dst: &str) -> Result<(), String> {
    let content = fs::read(src).map_err(|e| format!("读取文件失败: {e}"))?;
    let fname = Path::new(src).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "file.pdf".into());
    // 构造 multipart/form-data
    let boundary = format!("----PS{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    let mut body = Vec::new();
    for (k, v) in [("source", "auto"), ("target", "zh-CN")] {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{fname}\"\r\nContent-Type: application/pdf\r\n\r\n").as_bytes());
    body.extend_from_slice(&content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let resp = ureq::post("https://translate.fast-api-hub.top/api/translate-file")
        .set("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
        .send_bytes(&body)
        .map_err(|e| format!("云端转换失败: {e}"))?;
    let bytes = resp.into_string().map_err(|e| format!("读取响应失败: {e}"))?;
    // 响应可能是 docx 二进制或 JSON 错误
    if bytes.starts_with("PK") || bytes.len() > 1000 {
        fs::write(dst, &bytes).map_err(|e| format!("保存失败: {e}"))?;
        Ok(())
    } else {
        Err(format!("云端返回异常: {}", bytes.chars().take(200).collect::<String>()))
    }
}

fn run_pandoc(src: &str, dst: &str) -> Result<(), String> {
    let bin = find_bin(&["pandoc"]);
    let b = bin.ok_or("未找到 pandoc（转换引擎缺失）")?;
    let out = std::process::Command::new(b).args([src, "-o", dst]).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).into_owned()) }
}

fn run_pillow(src: &str, dst: &str) -> Result<(), String> {
    // V2: 改用 Rust 原生 image crate，不再依赖电脑装 Python/PIL
    let img = image::open(src).map_err(|e| format!("读取图片失败: {e}"))?;
    let ext = Path::new(dst).extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
    match ext.as_str() {
        "png" => img.save(dst).map_err(|e| format!("保存失败: {e}")),
        "jpg" | "jpeg" => img.save(dst).map_err(|e| format!("保存失败: {e}")),
        "webp" => img.save(dst).map_err(|e| format!("保存失败: {e}")),
        "bmp" => img.save(dst).map_err(|e| format!("保存失败: {e}")),
        "gif" => img.save(dst).map_err(|e| format!("保存失败: {e}")),
        _ => Err(format!("不支持的图片目标格式: {ext}")),
    }
}

fn run_ffmpeg(src: &str, dst: &str) -> Result<(), String> {
    let bin = find_bin(&["ffmpeg"]).ok_or("未找到 ffmpeg（转换引擎缺失）")?;
    let out = std::process::Command::new(bin).args(["-y", "-i", src, dst]).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).into_owned()) }
}

fn find_bin(names: &[&str]) -> Option<String> {
    for n in names {
        if let Ok(out) = std::process::Command::new(n).arg("--version").output() {
            if out.status.success() { return Some(n.to_string()); }
        }
    }
    None
}

// ============ V2.4 区域锁定翻译 ============
// 用户框选屏幕区域 → 只盯该区域：滚动中不翻（FrameGate 帧门控），
// 停稳后连续文本稳定才交付（TemporalTextTracker 稳定器），译文显示在区域下方悬浮条
const OCR_API: &str = "https://ocr.fast-api-hub.top/ocr-lines-b64";
static OCR_ENABLED: AtomicBool = AtomicBool::new(false);
static OCR_REGION: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);

#[tauri::command]
fn set_ocr_enabled(enabled: bool) {
    OCR_ENABLED.store(enabled, Ordering::Relaxed);
    log_ocr(&format!("屏幕自动翻译: {}", if enabled { "开" } else { "关" }));
}

#[tauri::command]
fn set_ocr_region(x: i32, y: i32, w: i32, h: i32) {
    if w >= 50 && h >= 20 {
        *OCR_REGION.lock().unwrap() = Some((x, y, w, h));
        // 持久化，重启后自动恢复
        let _ = fs::write(
            "ocr_region.json",
            serde_json::json!({"x": x, "y": y, "w": w, "h": h}).to_string(),
        );
        log_ocr(&format!("翻译区域: ({x},{y}) {w}x{h}"));
    }
}

#[tauri::command]
fn get_ocr_region() -> Vec<i32> {
    match *OCR_REGION.lock().unwrap() {
        Some((x, y, w, h)) => vec![x, y, w, h],
        None => vec![],
    }
}

#[tauri::command]
fn clear_ocr_region() {
    *OCR_REGION.lock().unwrap() = None;
    let _ = fs::remove_file("ocr_region.json");
    log_ocr("翻译区域: 已清除");
}

// 打开全屏区域选择器
#[tauri::command]
fn open_region_picker(app: AppHandle) {
    if let Some(w) = app.get_webview_window("region-picker") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

// 关闭区域选择器（框选完成或取消）
#[tauri::command]
fn close_region_picker(app: AppHandle) {
    if let Some(w) = app.get_webview_window("region-picker") {
        let _ = w.hide();
    }
}

fn load_ocr_region() {
    // 启动时从文件恢复上次框选的区域
    if let Ok(s) = fs::read_to_string("ocr_region.json") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
            let x = v["x"].as_i64().unwrap_or(0) as i32;
            let y = v["y"].as_i64().unwrap_or(0) as i32;
            let w = v["w"].as_i64().unwrap_or(0) as i32;
            let h = v["h"].as_i64().unwrap_or(0) as i32;
            if w >= 50 && h >= 20 {
                *OCR_REGION.lock().unwrap() = Some((x, y, w, h));
            }
        }
    }
}

fn ocr_enabled() -> bool {
    OCR_ENABLED.load(Ordering::Relaxed)
}

fn log_ocr(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true).append(true).open("ocr_log.txt")
    {
        let _ = writeln!(f, "[{}] {msg}", chrono_local_now());
    }
}

fn chrono_local_now() -> String {
    // 轻量本地时间（无 chrono 依赖：用 std::time）
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn capture_screen() -> Result<(i32, i32, image::RgbaImage), String> {
    #[cfg(target_os = "windows")]
    {
        // 整屏截图（PowerShell + System.Drawing，与 screen_capture_region 同法）
        let ps = "Add-Type -AssemblyName System.Drawing; Add-Type -AssemblyName System.Windows.Forms; \
                  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; \
                  $w = $b.Width; $h = $b.Height; \
                  $bm = New-Object System.Drawing.Bitmap($w,$h); \
                  $g = [System.Drawing.Graphics]::FromImage($bm); \
                  $g.CopyFromScreen($b.X,$b.Y,0,0,($w,$h)); \
                  $ms = New-Object System.IO.MemoryStream; \
                  $bm.Save($ms,[System.Drawing.Imaging.ImageFormat]::Png); \
                  [Convert]::ToBase64String($ms.ToArray())";
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", ps])
            .output()
            .map_err(|e| format!("截屏失败: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        let b64 = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let bytes = base64_decode(&b64)?;
        // 解码 PNG → RgbaImage
        let img = image::load_from_memory(&bytes)
            .map_err(|e| format!("PNG解码失败: {e}"))?
            .to_rgba8();
        let (w, h) = (img.width() as i32, img.height() as i32);
        Ok((w, h, img))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("非 Windows 平台".into())
    }
}

// 区域截屏：PowerShell 只截框选区域（比整屏快，减少传输）
fn capture_region_area(x: i32, y: i32, w: i32, h: i32) -> Result<image::RgbaImage, String> {
    #[cfg(target_os = "windows")]
    {
        let ps = format!(
            "Add-Type -AssemblyName System.Drawing; \
             $bm = New-Object System.Drawing.Bitmap({w},{h}); \
             $g = [System.Drawing.Graphics]::FromImage($bm); \
             $g.CopyFromScreen({x},{y},0,0,(New-Object System.Drawing.Size({w},{h}))); \
             $ms = New-Object System.IO.MemoryStream; \
             $bm.Save($ms,[System.Drawing.Imaging.ImageFormat]::Png); \
             [Convert]::ToBase64String($ms.ToArray())"
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .output()
            .map_err(|e| format!("区域截屏失败: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        let b64 = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let bytes = base64_decode(&b64)?;
        image::load_from_memory(&bytes)
            .map_err(|e| format!("PNG解码失败: {e}"))
            .map(|img| img.to_rgba8())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("非 Windows 平台".into())
    }
}

// ---- 缩略图：保持宽高比缩到 ≤256x128（DeskTranslate FrameGate 同款）----
fn thumbnail_gray(img: &image::RgbaImage) -> image::GrayImage {
    let gray = image::imageops::grayscale(img);
    let (w, h) = (gray.width(), gray.height());
    let scale = (256.0 / w.max(1) as f64).min(128.0 / h.max(1) as f64).min(1.0);
    let (nw, nh) = (
        ((w as f64 * scale).max(1.0) as u32).max(1),
        ((h as f64 * scale).max(1.0) as u32).max(1),
    );
    image::imageops::resize(&gray, nw, nh, image::imageops::FilterType::Triangle)
}

// ---- FrameGate 帧门控（移植 DeskTranslate algorithms.py）----
// 核心：滚动/移动中绝不触发翻译；画面停稳 settle 秒后才 ready。
// 缩略图 diff 判断变化：均值差 > 阈值 或 显著像素比例 > 0.8% 或 小文字变化(≥4px)。
#[derive(Clone, Copy)]
struct FrameDecision {
    changed: bool,
    ready: bool,
    moving: bool,
}

struct FrameGate {
    settle: f64,          // 停稳判定时长
    threshold: f64,       // 灰度均值差阈值
    motion_interval: f64, // 移动判定窗口
    candidate: Option<image::GrayImage>,
    since: f64,
    emitted: bool,
    moving: bool,
    unstable_since: Option<f64>,
    last_sample: f64,
    recent_changes: std::collections::VecDeque<f64>,
}

impl FrameGate {
    fn new(settle: f64, threshold: f64, motion_interval: f64) -> Self {
        Self {
            settle,
            threshold,
            motion_interval,
            candidate: None,
            since: f64::NEG_INFINITY,
            emitted: false,
            moving: false,
            unstable_since: None,
            last_sample: f64::NEG_INFINITY,
            recent_changes: std::collections::VecDeque::new(),
        }
    }

    fn observe(&mut self, img: &image::RgbaImage, now: f64) -> FrameDecision {
        let sample = thumbnail_gray(img);
        let (tw, th) = (sample.width(), sample.height());
        let changed = match &self.candidate {
            None => true,
            Some(c) => {
                if c.width() != tw || c.height() != th {
                    true
                } else {
                    // 逐像素 diff 统计
                    let mut mean_sum = 0.0f64;
                    let mut significant = 0u32;
                    for y in 0..th {
                        for x in 0..tw {
                            let d = (c.get_pixel(x, y)[0] as i32 - sample.get_pixel(x, y)[0] as i32)
                                .unsigned_abs();
                            mean_sum += d as f64;
                            if d >= 20 {
                                significant += 1;
                            }
                        }
                    }
                    let total = (tw * th).max(1) as f64;
                    let mean = mean_sum / total;
                    let sig_ratio = significant as f64 / total;
                    // 短行/标点变化可远低于区域 0.8%，保留 ≥4px 的明显对比变化
                    let small_text = significant >= 4;
                    mean > self.threshold || sig_ratio > 0.008 || small_text
                }
            }
        };
        if changed {
            if self.motion_interval > 0.0 {
                self.recent_changes.push_back(now);
                while let Some(front) = self.recent_changes.front() {
                    if now - *front > 2.5 {
                        self.recent_changes.pop_front();
                    } else {
                        break;
                    }
                }
                // 2.5 秒内 ≥3 次变化 → 判定在滚动/移动
                if self.recent_changes.len() >= 3 {
                    self.moving = true;
                }
            }
            if self.unstable_since.is_none() {
                self.unstable_since = Some(now);
            }
            self.candidate = Some(sample);
            self.since = now;
            self.emitted = false;
        }
        // quiet = 距上次变化 ≥ settle 秒
        let mut quiet = now - self.since >= self.settle;
        if self.moving {
            // 移动中要求更久稳定（防滚动间隙误触发）
            quiet = quiet && now - self.since >= 1.0_f64.max(self.settle * 2.0);
        }
        let mut ready = quiet && (!self.emitted || self.moving);
        if quiet {
            self.moving = false;
            self.unstable_since = None;
        } else if self.motion_interval > 0.0 {
            if let Some(us) = self.unstable_since {
                if now - us >= self.motion_interval {
                    self.moving = true;
                    ready = now - self.last_sample >= self.motion_interval;
                }
            }
        }
        if ready {
            self.emitted = true;
            self.last_sample = now;
        }
        FrameDecision {
            changed,
            ready,
            moving: self.moving,
        }
    }
}

// ---- TemporalTextTracker 文本稳定器（移植 DeskTranslate temporal.py）----
// 同一文本连续多帧出现且不再增长才交付，防止 OCR 抖动导致同一句反复翻译。
struct Observation {
    text: String,
    at: f64,
}

struct TemporalTextTracker {
    settle: f64,
    history: std::collections::VecDeque<Observation>,
    candidate: String,
    accepted: String,
    repeated: usize,
    growing: bool,
}

impl TemporalTextTracker {
    fn new(settle: f64) -> Self {
        Self {
            settle,
            history: std::collections::VecDeque::new(),
            candidate: String::new(),
            accepted: String::new(),
            repeated: 0,
            growing: false,
        }
    }

    fn observe(&mut self, text: &str, now: f64) -> Option<String> {
        let t = text.trim().to_string();
        // 清除 2.5 秒前的历史
        while let Some(front) = self.history.front() {
            if now - front.at > 2.5 {
                self.history.pop_front();
            } else {
                break;
            }
        }
        if t.is_empty() {
            self.history.clear();
            self.candidate.clear();
            self.accepted.clear();
            self.repeated = 0;
            self.growing = false;
            return None;
        }
        let prev_text = self
            .history
            .back()
            .map(|o| o.text.clone())
            .unwrap_or_default();
        // 增长检测：新文本是旧文本前缀且更长 → 渐进式文字（如打字中）
        let growth = !prev_text.is_empty() && t.starts_with(&prev_text) && t.len() > prev_text.len();
        if t != self.candidate {
            self.repeated = 1;
        } else {
            self.repeated += 1;
        }
        if growth {
            self.growing = true;
        } else if !prev_text.is_empty() && t != prev_text && !small_text_change(&t, &prev_text) {
            self.growing = false;
        }
        self.candidate = t.clone();
        self.history.push_back(Observation { text: t.clone(), at: now });
        // 交付条件：同一文本连续 ≥3 帧（repeated≥3）且不再增长，且与已交付不同
        if self.repeated >= 3 && !self.growing && t != self.accepted {
            self.accepted = t.clone();
            return Some(t);
        }
        None
    }
}

fn small_text_change(left: &str, right: &str) -> bool {
    if !(4..=256).contains(&left.len().max(right.len())) || left.len().abs_diff(right.len()) > 2 {
        return false;
    }
    let changes = left
        .chars()
        .zip(right.chars())
        .filter(|(a, b)| a != b)
        .count()
        + left.len().abs_diff(right.len());
    changes <= left.len().min(right.len()).max(2) / 6 + 1
}

// 区域 PNG → base64
fn region_b64(img: &image::RgbaImage, x: i32, y: i32, w: i32, h: i32) -> Result<String, String> {
    let crop = image::imageops::crop_imm(img, x as u32, y as u32, w as u32, h as u32).to_image();
    let mut buf = std::io::Cursor::new(Vec::new());
    crop.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("PNG编码失败: {e}"))?;
    use base64::Engine as _;
    Ok(base64::engine::general_purpose::STANDARD.encode(buf.get_ref()))
}

#[derive(Deserialize)]
struct OcrLinesResp {
    ok: bool,
    #[serde(default)]
    lines: Vec<OcrLine>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    translated: String,
}
#[derive(Deserialize, Clone)]
struct OcrLine {
    text: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

// 自动 OCR 区域锁定翻译主循环（移植 DeskTranslate 机制）
fn ocr_translate_loop(app: AppHandle) {
    // 帧门控：0.3s 停稳判定，1.2 灰度差阈值，1.5s 移动判定窗口
    let mut gate = FrameGate::new(0.3, 1.2, 1.5);
    // 文本稳定器：同一文本连续 ≥3 帧才交付
    let mut stabilizer = TemporalTextTracker::new(0.25);
    let mut last_delivered = String::new();
    loop {
        std::thread::sleep(Duration::from_millis(300));
        // 开关关闭时不截屏（省资源）
        if !ocr_enabled() {
            gate = FrameGate::new(0.3, 1.2, 1.5);
            stabilizer = TemporalTextTracker::new(0.25);
            last_delivered.clear();
            continue;
        }
        // 取框选区域；未框选则静默
        let region = *OCR_REGION.lock().unwrap();
        let Some((rx, ry, rw, rh)) = region else { continue };
        if rw < 50 || rh < 20 {
            continue;
        }
        // 只截框选区域
        let Ok(img) = capture_region_area(rx, ry, rw, rh) else { continue };
        let now = now_secs();
        // 帧门控：滚动/移动中不 ready，停稳才 ready
        let decision = gate.observe(&img, now);
        if !decision.ready {
            continue;
        }
        // 区域图已裁剪，从 (0,0) 编码；服务端加回屏幕偏移 (rx, ry)
        let b64 = match region_b64(&img, 0, 0, rw, rh) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let body = serde_json::json!({ "image_b64": b64, "x": rx, "y": ry });
        let resp = match ureq::post(OCR_API)
            .timeout(Duration::from_secs(30))
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
        {
            Ok(r) => r,
            Err(_) => continue,
        };
        let resp_str = match resp.into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let parsed: OcrLinesResp = match serde_json::from_str(&resp_str) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if !parsed.ok || parsed.lines.is_empty() || parsed.text.trim().is_empty() {
            continue;
        }
        // 文本稳定器：连续 ≥3 帧同一文本（不再增长）才交付
        let Some(delivered) = stabilizer.observe(&parsed.text, now) else {
            continue;
        };
        if delivered == last_delivered {
            continue;
        }
        last_delivered = delivered.clone();
        log_ocr(&format!("交付译文: {}", parsed.translated));
        // 译文发到 ocr-overlay，显示在区域下方悬浮条（不盖原文）
        let lines: Vec<(String, i32, i32, i32, i32)> = parsed
            .lines
            .iter()
            .map(|l| (l.text.clone(), l.x, l.y, l.w, l.h))
            .collect();
        if let Some(w) = app.get_webview_window("ocr-overlay") {
            let _ = w.show();
        }
        let _ = app.emit("ocr-translate-result", serde_json::json!({
            "lines": lines,
            "translated": parsed.translated,
            "text": delivered,
            "region": { "x": rx, "y": ry, "w": rw, "h": rh },
        }));
    }
}

// ============ 入口 ============
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // 恢复上次框选的 OCR 翻译区域
            load_ocr_region();
            // 后台线程：轮询剪贴板，发现新复制文本 → 自动翻译 → 通知 overlay 悬浮窗
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                clipboard_monitor(handle);
            });
            // 后台线程：自动 OCR 区域锁定翻译（帧门控+稳定器 → 云端 OCR → 区域下方显示）
            let handle2 = app.handle().clone();
            std::thread::spawn(move || {
                ocr_translate_loop(handle2);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            minimize_window,
            close_window,
            translate,
            copy_text,
            speak_text,
            clipboard_poll,
            screen_capture_region,
            list_notebooks,
            create_notebook,
            list_pages,
            create_page,
            load_page,
            save_page,
            export_page,
            attach_file,
            voice_record,
            pick_file,
            convert_targets,
            convert_file,
            show_overlay,
            hide_overlay,
            quit_app,
            set_ocr_enabled,
            set_ocr_region,
            get_ocr_region,
            clear_ocr_region,
            open_region_picker,
            close_region_picker,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ---------- V2.1 沉浸式翻译：后台剪贴板监听 ----------
fn clipboard_monitor(app: AppHandle) {
    let mut last: Option<String> = None;
    loop {
        std::thread::sleep(Duration::from_millis(900));
        // 读剪贴板：用 arboard 专用实例（Windows/Linux 都可靠，不依赖主线程）
        let current = {
            let mut cb = match arboard::Clipboard::new() {
                Ok(c) => c,
                Err(_) => continue,
            };
            cb.get_text().ok()
        };
        let Some(text) = current else { continue };
        let text = text.trim().to_string();
        if text.is_empty() || text.len() < 2 || text.len() > 4000 {
            continue;
        }
        // 只处理非纯中文的外文（目标语言是中文）
        let has_foreign = text.chars().any(|c| c.is_ascii_alphabetic());
        if !has_foreign { continue; }
        // 内容没变 → 跳过（避免重复翻译）
        if let Some(l) = &last {
            if l == &text { continue; }
        }
        last = Some(text.clone());

        // 调用翻译
        match translate(text.clone(), "zh".into(), "auto".into()) {
            Ok(translated) => {
                if translated.is_empty() || translated == text { continue; }
                // 通知 overlay 窗口显示
                let _ = app.emit("translate-result", serde_json::json!({
                    "text": text,
                    "translated": translated,
                }));
                // 显示悬浮窗（不抢焦点，避免打断用户打字）
                if let Some(w) = app.get_webview_window("overlay") {
                    let _ = w.show();
                }
            }
            Err(_) => {}
        }
    }
}

// ---------- V2.2 屏幕框选 OCR 翻译 ----------
#[tauri::command]
fn show_overlay(app: AppHandle) {
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.show();
    }
}

#[tauri::command]
fn hide_overlay(app: AppHandle) {
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.hide();
    }
}
