// ============ 个人精灵 · Rust 后端 ============
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
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

// ============ 入口 ============
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // 后台线程：轮询剪贴板，发现新复制文本 → 自动翻译 → 通知 overlay 悬浮窗
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                clipboard_monitor(handle);
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
