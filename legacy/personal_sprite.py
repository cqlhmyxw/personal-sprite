#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
个人精灵 · Windows 桌面版
功能：翻译(文本/语音) + 个人笔记本 + 文件翻译(PDF/图片→Word)
技术：Python 标准库 tkinter + urllib（零第三方依赖）
云端：翻译网关 tapi.fast-api-hub.top / 文件翻译 translate.fast-api-hub.top
"""
import tkinter as tk
from tkinter import ttk, filedialog, messagebox, scrolledtext
import json, os, io, re, sys, time, threading, subprocess, tempfile, urllib.request, urllib.parse
from datetime import datetime

# ============ 全局配置 ============
APP_NAME = "个人精灵"
VERSION = "1.1.0"
API_BASE = "https://tapi.fast-api-hub.top"
FILE_API = "https://translate.fast-api-hub.top"
API_KEY = "xiaokong-translate-2026"
APP_DIR = os.path.join(os.path.expanduser("~"), "PersonalSprite")
NOTES_FILE = os.path.join(APP_DIR, "notes.json")
if not os.path.exists(APP_DIR):
    os.makedirs(APP_DIR, exist_ok=True)

LANGS = {"自动检测": "auto", "中文": "zh-CN", "英语": "en", "日语": "ja",
         "韩语": "ko", "法语": "fr", "德语": "de", "西班牙语": "es", "俄语": "ru"}
LANG_KEYS = {v: k for k, v in LANGS.items()}
# 注意：LANG_KEYS = {代码: 显示名}。按显示名取代码要用 LANGS.get(显示名)。
def lang_code(display_name, default="auto"):
    return LANGS.get(display_name, default)
VOICE_MAP = {"zh-CN": "zh-CN-XiaoxiaoNeural", "en": "en-US-AriaNeural",
             "ja": "ja-JP-NanamiNeural", "ko": "ko-KR-SunHiNeural"}

def http_post_json(url, fields, headers=None, timeout=60):
    """multipart/form-data POST，返回 (status, json)"""
    import uuid
    boundary = "----WebKitFormBoundary" + uuid.uuid4().hex
    body = io.BytesIO()
    for k, v in fields.items():
        body.write(f"--{boundary}\r\n".encode())
        if isinstance(v, tuple):  # 文件: (filename, content, mime)
            fname, content, mime = v
            body.write(f'Content-Disposition: form-data; name="{k}"; filename="{fname}"\r\n'.encode())
            body.write(f"Content-Type: {mime}\r\n\r\n".encode())
            body.write(content)
            body.write(b"\r\n")
        else:
            body.write(f'Content-Disposition: form-data; name="{k}"\r\n\r\n'.encode())
            body.write(str(v).encode())
            body.write(b"\r\n")
    body.write(f"--{boundary}--\r\n".encode())
    h = {"Content-Type": f"multipart/form-data; boundary={boundary}",
         "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) PersonalSprite/1.0"}
    if headers:
        h.update(headers)
    req = urllib.request.Request(url, data=body.getvalue(), headers=h, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read().decode())
        except Exception:
            return e.code, {"detail": f"HTTP {e.code}"}
    except Exception as e:
        return 0, {"detail": str(e)}

def translate(text, target="zh-CN", source="auto"):
    status, data = http_post_json(f"{API_BASE}/api/translate",
        {"text": text, "target": target, "source": source},
        headers={"X-API-Key": API_KEY})
    if status == 200:
        return data.get("translated_text", "")
    return f"[翻译失败] {data.get('detail', '')}"

# ============ 笔记本存储 ============
def load_notes():
    try:
        with open(NOTES_FILE, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return []

def save_notes(notes):
    with open(NOTES_FILE, "w", encoding="utf-8") as f:
        json.dump(notes, f, ensure_ascii=False, indent=2)

# ============ 主窗口 ============
class PersonalSprite:
    def __init__(self, root):
        self.root = root
        self.root.title(f"{APP_NAME} v{VERSION}")
        self.root.geometry("860x640")
        self.root.minsize(720, 520)
        self.notes = load_notes()
        self._build_ui()
        self._load_notes_list()

    def _build_ui(self):
        # 顶栏：Logo + 状态
        header = tk.Frame(self.root, bg="#2b6cb0", height=48)
        header.pack(fill="x")
        tk.Label(header, text="🗿 个人精灵", font=("Microsoft YaHei", 15, "bold"),
                 bg="#2b6cb0", fg="white").pack(side="left", padx=16, pady=8)
        self.status_lbl = tk.Label(header, text="就绪", font=("Microsoft YaHei", 9),
                                   bg="#2b6cb0", fg="#cde3f7")
        self.status_lbl.pack(side="right", padx=16)

        # Tab 容器
        self.tab = ttk.Notebook(self.root)
        self.tab.pack(fill="both", expand=True, padx=8, pady=8)
        self._build_translate_tab()
        self._build_notes_tab()
        self._build_file_tab()

    def _set_status(self, msg):
        self.status_lbl.config(text=msg)
        self.root.update_idletasks()

    # ---------- Tab 1: 翻译 ----------
    def _build_translate_tab(self):
        tab = ttk.Frame(self.tab)
        self.tab.add(tab, text="  🗣️ 翻译  ")

        # 语言选择行
        row = tk.Frame(tab); row.pack(fill="x", padx=12, pady=(12, 4))
        tk.Label(row, text="源语言:", font=("Microsoft YaHei", 10)).pack(side="left")
        self.src_lang = ttk.Combobox(row, values=list(LANGS.keys()), width=10, state="readonly")
        self.src_lang.current(0); self.src_lang.pack(side="left", padx=(2, 10))
        tk.Label(row, text="→", font=("Microsoft YaHei", 12, "bold")).pack(side="left")
        self.dst_lang = ttk.Combobox(row, values=list(LANGS.keys()), width=10, state="readonly")
        self.dst_lang.current(1); self.dst_lang.pack(side="left", padx=(10, 0))
        tk.Button(row, text="互换", command=self._swap_lang).pack(side="left", padx=10)

        # 输入区
        tk.Label(tab, text="输入文字:", font=("Microsoft YaHei", 10)).pack(anchor="w", padx=12, pady=(8, 0))
        self.trans_in = scrolledtext.ScrolledText(tab, height=6, font=("Microsoft YaHei", 11),
                                                   relief="solid", bd=1)
        self.trans_in.pack(fill="x", padx=12, pady=4)

        # 按钮行
        btn_row = tk.Frame(tab); btn_row.pack(fill="x", padx=12)
        tk.Button(btn_row, text="🔍 翻译", command=self._do_translate,
                  font=("Microsoft YaHei", 11, "bold"), bg="#2b6cb0", fg="white",
                  width=12).pack(side="left", pady=6)
        tk.Button(btn_row, text="🔊 朗读译文", command=self._speak_translation,
                  font=("Microsoft YaHei", 10)).pack(side="left", padx=8)
        tk.Button(btn_row, text="📋 复制结果", command=lambda: self._copy_text(self.trans_out),
                  font=("Microsoft YaHei", 10)).pack(side="left", padx=4)

        # 结果区
        tk.Label(tab, text="翻译结果:", font=("Microsoft YaHei", 10)).pack(anchor="w", padx=12, pady=(8, 0))
        self.trans_out = scrolledtext.ScrolledText(tab, height=6, font=("Microsoft YaHei", 11),
                                                    relief="solid", bd=1, bg="#f8f9fa")
        self.trans_out.pack(fill="both", expand=True, padx=12, pady=4)

        # 语音翻译提示
        tk.Label(tab, text="语音翻译: 用系统语音输入（Win+H）后点翻译即可自动识别",
                 font=("Microsoft YaHei", 9), fg="#888").pack(anchor="w", padx=12, pady=(2, 8))

    def _swap_lang(self):
        a, b = self.src_lang.get(), self.dst_lang.get()
        self.src_lang.set(b); self.dst_lang.set(a)

    def _do_translate(self):
        text = self.trans_in.get("1.0", "end").strip()
        if not text:
            messagebox.showinfo(APP_NAME, "请输入要翻译的文字")
            return
        self._set_status("翻译中…")
        src, dst = lang_code(self.src_lang.get(), "auto"), lang_code(self.dst_lang.get(), "zh-CN")
        # 让用户在状态栏明确看到实际请求方向，避免“看起来选中文，实际传错代码”
        self._set_status(f"翻译中：{self.src_lang.get()} → {self.dst_lang.get()}")
        def work():
            result = translate(text, dst, src if src != "auto" else "auto")
            self.root.after(0, lambda: self._finish_translate(result))
        threading.Thread(target=work, daemon=True).start()

    def _finish_translate(self, result):
        self.trans_out.delete("1.0", "end")
        self.trans_out.insert("1.0", result)
        self._set_status("翻译完成 ✓")

    def _speak_translation(self):
        text = self.trans_out.get("1.0", "end").strip()
        if not text:
            messagebox.showinfo(APP_NAME, "没有可朗读的译文")
            return
        dst = lang_code(self.dst_lang.get(), "zh-CN")
        voice = VOICE_MAP.get(dst, "zh-CN-XiaoxiaoNeural")
        def work2():
            try:
                boundary = "----PB" + uuid_hex()
                body = f"--{boundary}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{text}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"voice\"\r\n\r\n{voice}\r\n--{boundary}--\r\n".encode()
                req = urllib.request.Request(f"{API_BASE}/api/tts",
                    data=body,
                    headers={"X-API-Key": API_KEY,
                             "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) PersonalSprite/1.0",
                             "Content-Type": f"multipart/form-data; boundary={boundary}"})
                with urllib.request.urlopen(req, timeout=30) as resp:
                    mp3 = resp.read()
                tmp = os.path.join(tempfile.gettempdir(), "sprite_tts.mp3")
                with open(tmp, "wb") as f2:
                    f2.write(mp3)
                if sys.platform == "win32":
                    os.startfile(tmp)  # 用默认播放器
            except Exception as e:
                self.root.after(0, lambda: messagebox.showerror(APP_NAME, f"朗读失败: {e}"))
        threading.Thread(target=work2, daemon=True).start()

    def _copy_text(self, widget):
        text = widget.get("1.0", "end").strip()
        if text:
            self.root.clipboard_clear()
            self.root.clipboard_append(text)
            self._set_status("已复制 ✓")

    # ---------- Tab 2: 笔记本 ----------
    def _build_notes_tab(self):
        tab = ttk.Frame(self.tab)
        self.tab.add(tab, text="  📝 笔记本  ")

        # 输入区
        self.note_in = scrolledtext.ScrolledText(tab, height=5, font=("Microsoft YaHei", 11),
                                                  relief="solid", bd=1)
        self.note_in.pack(fill="x", padx=12, pady=(12, 4))
        btn_row = tk.Frame(tab); btn_row.pack(fill="x", padx=12)
        tk.Button(btn_row, text="💾 保存记录", command=self._save_note,
                  font=("Microsoft YaHei", 11, "bold"), bg="#27ae60", fg="white",
                  width=12).pack(side="left")
        tk.Button(btn_row, text="🔍 搜索", command=self._search_notes,
                  font=("Microsoft YaHei", 10)).pack(side="left", padx=8)
        tk.Button(btn_row, text="🔁 全部", command=self._load_notes_list,
                  font=("Microsoft YaHei", 10)).pack(side="left")
        tk.Button(btn_row, text="📤 导出 TXT", command=self._export_notes,
                  font=("Microsoft YaHei", 10)).pack(side="right")

        # 列表区
        tk.Label(tab, text="我的记录:", font=("Microsoft YaHei", 10)).pack(anchor="w", padx=12, pady=(8, 0))
        self.note_list_frame = tk.Frame(tab)
        self.note_list_frame.pack(fill="both", expand=True, padx=12, pady=4)

    def _save_note(self):
        txt = self.note_in.get("1.0", "end").strip()
        if not txt:
            messagebox.showinfo(APP_NAME, "写点什么吧")
            return
        now = datetime.now().strftime("%Y-%m-%d %H:%M")
        self.notes.insert(0, {"time": now, "text": txt, "tags": []})
        save_notes(self.notes)
        self.note_in.delete("1.0", "end")
        self._load_notes_list()
        self._set_status("已保存 ✓")

    def _load_notes_list(self, notes=None):
        for w in self.note_list_frame.winfo_children():
            w.destroy()
        notes = notes if notes is not None else self.notes
        if not notes:
            tk.Label(self.note_list_frame, text="还没有记录，写点什么吧",
                     font=("Microsoft YaHei", 10), fg="#999").pack(anchor="w", pady=8)
            return
        # 显示最近 50 条
        for i, n in enumerate(notes[:50]):
            card = tk.Frame(self.note_list_frame, bg="#f4f6f8", highlightthickness=1,
                            highlightbackground="#e0e4e8")
            card.pack(fill="x", pady=2)
            head = tk.Frame(card, bg="#f4f6f8")
            head.pack(fill="x", padx=8, pady=(4, 0))
            tk.Label(head, text=n["time"], font=("Microsoft YaHei", 8), fg="#999",
                     bg="#f4f6f8").pack(side="left")
            tk.Button(head, text="✕ 删除", font=("Microsoft YaHei", 8), bd=0, fg="#c0392b",
                      bg="#f4f6f8", cursor="hand2",
                      command=lambda idx=i: self._del_note(idx)).pack(side="right")
            body_txt = n["text"] if len(n["text"]) <= 120 else n["text"][:120] + "…"
            tk.Label(card, text=body_txt, font=("Microsoft YaHei", 10),
                     bg="#f4f6f8", anchor="w", justify="left", wraplength=720).pack(
                fill="x", padx=8, pady=(0, 6))

    def _del_note(self, idx):
        if 0 <= idx < len(self.notes):
            self.notes.pop(idx)
            save_notes(self.notes)
            self._load_notes_list()

    def _search_notes(self):
        kw = ""
        # 简单搜索：弹输入框
        dlg = tk.Toplevel(self.root)
        dlg.title("搜索记录")
        dlg.geometry("380x100")
        tk.Label(dlg, text="关键词:", font=("Microsoft YaHei", 10)).pack(pady=(12, 4))
        e = tk.Entry(dlg, font=("Microsoft YaHei", 11), width=30)
        e.pack()
        e.focus_set()
        def go():
            kw = e.get().strip().lower()
            dlg.destroy()
            if kw:
                r = [n for n in self.notes if kw in n["text"].lower()]
                self._load_notes_list(r)
                self._set_status(f"搜索 '{kw}'：{len(r)} 条")
        tk.Button(dlg, text="搜索", command=go, font=("Microsoft YaHei", 10)).pack(pady=6)
        dlg.bind("<Return>", lambda ev: go())

    def _export_notes(self):
        if not self.notes:
            messagebox.showinfo(APP_NAME, "没有可导出的记录")
            return
        path = filedialog.asksaveasfilename(defaultextension=".txt",
            filetypes=[("文本文件", "*.txt")], initialfile="个人精灵笔记.txt")
        if not path:
            return
        try:
            with open(path, "w", encoding="utf-8") as f:
                for n in self.notes:
                    f.write(f"[{n['time']}]\n{n['text']}\n\n")
            self._set_status(f"已导出 {len(self.notes)} 条 → {os.path.basename(path)}")
        except Exception as e:
            messagebox.showerror(APP_NAME, f"导出失败: {e}")

    # ---------- Tab 3: 文件翻译 ----------
    def _build_file_tab(self):
        tab = ttk.Frame(self.tab)
        self.tab.add(tab, text="  📄 文件翻译  ")

        tk.Label(tab, text="上传 PDF 或图片，翻译后保存为 Word 文档",
                 font=("Microsoft YaHei", 10), fg="#555").pack(anchor="w", padx=12, pady=(12, 6))
        row = tk.Frame(tab); row.pack(fill="x", padx=12)
        tk.Button(row, text="📁 选择文件…", command=self._pick_file,
                  font=("Microsoft YaHei", 11), width=14).pack(side="left")
        self.file_lbl = tk.Label(row, text="未选择文件", font=("Microsoft YaHei", 10), fg="#999")
        self.file_lbl.pack(side="left", padx=10)

        row2 = tk.Frame(tab); row2.pack(fill="x", padx=12, pady=8)
        tk.Label(row2, text="目标语言:", font=("Microsoft YaHei", 10)).pack(side="left")
        self.file_lang = ttk.Combobox(row2, values=["简体中文", "英语", "日语", "韩语"],
                                       width=10, state="readonly")
        self.file_lang.current(0); self.file_lang.pack(side="left", padx=(4, 12))
        tk.Button(row2, text="🚀 开始翻译", command=self._do_file_translate,
                  font=("Microsoft YaHei", 11, "bold"), bg="#2b6cb0", fg="white").pack(side="left")

        tk.Label(tab, text="翻译结果:", font=("Microsoft YaHei", 10)).pack(anchor="w", padx=12, pady=(8, 0))
        self.file_out = scrolledtext.ScrolledText(tab, height=12, font=("Microsoft YaHei", 10),
                                                   relief="solid", bd=1, bg="#f8f9fa")
        self.file_out.pack(fill="both", expand=True, padx=12, pady=4)
        self.file_out.insert("1.0", "选择文件后点击\"开始翻译\"，会自动转换并保存 Word 文档。")

        self._picked_file = None

    def _pick_file(self):
        path = filedialog.askopenfilename(
            filetypes=[("PDF/图片", "*.pdf *.png *.jpg *.jpeg *.webp")])
        if path:
            self._picked_file = path
            self.file_lbl.config(text=os.path.basename(path), fg="#2b6cb0")
            self.file_out.delete("1.0", "end")
            self.file_out.insert("1.0", f"已选择: {os.path.basename(path)}\n点击\"开始翻译\"。")

    def _do_file_translate(self):
        if not self._picked_file or not os.path.exists(self._picked_file):
            messagebox.showinfo(APP_NAME, "请先选择文件")
            return
        target_map = {"简体中文": "zh-CN", "英语": "en", "日语": "ja", "韩语": "ko"}
        target = target_map.get(self.file_lang.get(), "zh-CN")
        path = self._picked_file
        fname = os.path.basename(path)
        self._set_status("上传翻译中…")
        self.file_out.delete("1.0", "end")
        self.file_out.insert("1.0", "⏳ 上传并翻译中（OCR + 翻译，约 10-30 秒）…\n")

        def work():
            try:
                with open(path, "rb") as f:
                    content = f.read()
                ext = os.path.splitext(fname)[1].lower()
                mime = {"pdf": "application/pdf", "png": "image/png",
                        "jpg": "image/jpeg", "jpeg": "image/jpeg", "webp": "image/webp"}.get(ext.lstrip("."), "application/octet-stream")
                boundary = "----PB" + uuid_hex()
                body = io.BytesIO()
                for k, v in [("source", "auto"), ("target", target)]:
                    body.write(f"--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n".encode())
                body.write(f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{fname}\"\r\nContent-Type: {mime}\r\n\r\n".encode())
                body.write(content)
                body.write(f"\r\n--{boundary}--\r\n".encode())
                req = urllib.request.Request(f"{FILE_API}/api/translate-file",
                    data=body.getvalue(),
                    headers={"Content-Type": f"multipart/form-data; boundary={boundary}",
                             "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) PersonalSprite/1.0"})
                with urllib.request.urlopen(req, timeout=180) as resp:
                    docx_bytes = resp.read()
                # 自动保存到固定目录（不弹对话框，最省心）
                save_dir = os.path.join(APP_DIR, "翻译结果")
                os.makedirs(save_dir, exist_ok=True)
                stamp = datetime.now().strftime("%Y%m%d_%H%M%S")
                save_path = os.path.join(save_dir, f"翻译_{os.path.splitext(fname)[0]}_{stamp}.docx")
                with open(save_path, "wb") as f:
                    f.write(docx_bytes)
                # 尝试自动打开所在文件夹
                self.root.after(0, lambda p=save_path: self._finish_file_translate(True, f"✅ 翻译完成！\n已保存到:\n{p}\n\n（可点击右上角文件夹图标查看）"))
                self.root.after(0, lambda p=save_path: self._open_folder(p))
            except Exception as e:
                self.root.after(0, lambda: self._finish_file_translate(False, f"翻译失败: {e}"))
        threading.Thread(target=work, daemon=True).start()

    def _open_folder(self, path):
        """自动打开文件所在文件夹（Windows）"""
        try:
            import subprocess
            folder = os.path.dirname(path)
            subprocess.Popen(["explorer", folder])
        except Exception:
            pass

    def _finish_file_translate(self, ok, msg):
        self.file_out.delete("1.0", "end")
        self.file_out.insert("1.0", msg)
        self._set_status("翻译完成 ✓" if ok else "翻译失败 ✗")

def uuid_hex():
    import uuid
    return uuid.uuid4().hex

def main():
    root = tk.Tk()
    try:
        style = ttk.Style()
        if "vista" in style.theme_names():
            style.theme_use("vista")
    except Exception:
        pass
    app = PersonalSprite(root)
    root.mainloop()

if __name__ == "__main__":
    main()