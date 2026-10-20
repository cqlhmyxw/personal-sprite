// ============ 个人精灵 · 前端逻辑 ============
import { invoke } from "@tauri-apps/api/core";

// ---------- 工具 ----------
const $ = (sel: string) => document.querySelector(sel) as HTMLElement;
const $$ = (sel: string) => Array.from(document.querySelectorAll(sel)) as HTMLElement[];
const state = {
  currentNb: "",
  currentPage: "",
  currentFile: null as string | null,
};

function setStatus(sel: string, msg: string, kind: "" | "ok" | "err" = "") {
  const el = $(sel);
  el.textContent = msg;
  el.className = "status-bar" + (kind ? " " + kind : "");
}

// ---------- Tab 切换 ----------
$$(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    $$(".tab-btn").forEach((b) => b.classList.remove("active"));
    $$(".tab-panel").forEach((p) => p.classList.remove("active"));
    btn.classList.add("active");
    $(`#tab-${btn.dataset.tab}`).classList.add("active");
  });
});

// ---------- 窗口控制 ----------
$("#btn-min").addEventListener("click", () => invoke("minimize_window"));
$("#btn-close").addEventListener("click", () => invoke("close_window"));

// ============ 翻译 ============

// V2.1 沉浸式翻译：由 Rust 后台线程监听剪贴板，自动翻译并显示在系统级悬浮窗(overlay)
// 本页只保留手动翻译按钮

// ============ V2.3 屏幕自动翻译开关 ============
const ocrToggle = $("#ocr-auto-toggle") as HTMLInputElement;
const OCR_KEY = "sprite_ocr_enabled";

// 恢复上次状态
try {
  const saved = localStorage.getItem(OCR_KEY);
  if (saved === "1") {
    ocrToggle.checked = true;
    invoke("set_ocr_enabled", { enabled: true });
  }
} catch { /* ignore */ }

ocrToggle.addEventListener("change", () => {
  const on = ocrToggle.checked;
  invoke("set_ocr_enabled", { enabled: on }).catch((e) => {
    console.log("set_ocr_enabled 失败:", e);
  });
  try {
    localStorage.setItem(OCR_KEY, on ? "1" : "0");
  } catch { /* ignore */ }
  ocrToggle.dispatchEvent(new Event("ocr-toggle-change"));
});

$("#btn-translate").addEventListener("click", async () => {
  const text = ($("#trans-in") as HTMLTextAreaElement).value.trim();
  if (!text) { setStatus("#trans-status", "请输入要翻译的文字", "err"); return; }
  const src = ($("#src-lang") as HTMLSelectElement).value;
  const dst = ($("#dst-lang") as HTMLSelectElement).value;
  setStatus("#trans-status", `翻译中…（${src} → ${dst}）`);
  try {
    const result = await invoke<string>("translate", { text, target: dst, source: src });
    ($("#trans-out") as HTMLTextAreaElement).value = result;
    setStatus("#trans-status", `✅ 已翻译（${src} → ${dst}）`, "ok");
  } catch (e) {
    setStatus("#trans-status", `翻译失败: ${e}`, "err");
  }
});

$("#btn-copy").addEventListener("click", async () => {
  const out = ($("#trans-out") as HTMLTextAreaElement).value;
  if (!out) return;
  try { await invoke("copy_text", { text: out }); setStatus("#trans-status", "✅ 已复制", "ok"); }
  catch (e) { setStatus("#trans-status", `复制失败: ${e}`, "err"); }
});

$("#btn-speak").addEventListener("click", async () => {
  const out = ($("#trans-out") as HTMLTextAreaElement).value;
  if (!out) return;
  try { await invoke("speak_text", { text: out }); }
  catch (e) { setStatus("#trans-status", `朗读失败: ${e}`, "err"); }
});

// ============ 笔记 ============
async function loadNotebooks() {
  const nbs = await invoke<Array<{ id: string; name: string }>>("list_notebooks");
  const ul = $("#nb-list");
  ul.innerHTML = "";
  nbs.forEach((nb) => {
    const li = document.createElement("li");
    li.textContent = "📓 " + nb.name;
    li.dataset.id = nb.id;
    if (nb.id === state.currentNb) li.classList.add("active");
    li.addEventListener("click", async () => {
      state.currentNb = nb.id;
      state.currentPage = "";
      $$("#nb-list li").forEach((x) => x.classList.remove("active"));
      li.classList.add("active");
      await loadPages();
      await loadPageContent("");
    });
    ul.appendChild(li);
  });
  if (!state.currentNb && nbs.length) {
    state.currentNb = nbs[0].id;
    $$("#nb-list li")[0]?.classList.add("active");
    loadPages();
  }
}

async function loadPages() {
  if (!state.currentNb) return;
  const pages = await invoke<Array<{ id: string; title: string }>>("list_pages", { notebookId: state.currentNb });
  const ul = $("#page-list");
  ul.innerHTML = "";
  pages.forEach((p) => {
    const li = document.createElement("li");
    li.textContent = "📄 " + (p.title || "未命名");
    li.dataset.id = p.id;
    if (p.id === state.currentPage) li.classList.add("active");
    li.addEventListener("click", async () => {
      state.currentPage = p.id;
      $$("#page-list li").forEach((x) => x.classList.remove("active"));
      li.classList.add("active");
      await loadPageContent(p.id);
    });
    ul.appendChild(li);
  });
}

async function loadPageContent(pageId: string) {
  if (!pageId) {
    ($("#note-title") as HTMLElement).textContent = "";
    ($("#note-body") as HTMLElement).textContent = "";
    return;
  }
  const page = await invoke<{ title: string; content: string }>("load_page", { pageId });
  ($("#note-title") as HTMLElement).textContent = page.title;
  ($("#note-body") as HTMLElement).innerHTML = page.content;
}

$("#btn-add-nb").addEventListener("click", async () => {
  const name = prompt("新笔记本名称：");
  if (!name) return;
  await invoke("create_notebook", { name });
  await loadNotebooks();
});

$("#btn-add-page").addEventListener("click", async () => {
  if (!state.currentNb) { alert("请先创建笔记本"); return; }
  const page = await invoke<{ id: string }>("create_page", { notebookId: state.currentNb });
  state.currentPage = page.id;
  await loadPages();
  await loadPageContent(page.id);
});

$("#btn-note-save").addEventListener("click", async () => {
  if (!state.currentPage) { alert("请先选择或新建页面"); return; }
  const title = ($("#note-title") as HTMLElement).textContent || "";
  const body = ($("#note-body") as HTMLElement).innerHTML;
  await invoke("save_page", { pageId: state.currentPage, title, content: body });
  await loadPages();
  alert("✅ 已保存");
});

// 工具栏
$$(".tool-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    const cmd = btn.dataset.cmd!;
    ($("#note-body") as HTMLElement).focus();
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0) return;
    const r = sel.getRangeAt(0);
    const wrap = (tag: string) => {
      const el = document.createElement(tag);
      el.appendChild(r.extractContents());
      r.insertNode(el);
    };
    if (cmd === "bold") wrap("b");
    else if (cmd === "italic") wrap("i");
    else if (cmd === "h1") wrap("h3");
    else if (cmd === "h2") wrap("h4");
    else if (cmd === "ul") {
      const ul = document.createElement("ul");
      const li = document.createElement("li");
      li.appendChild(r.extractContents());
      ul.appendChild(li);
      r.insertNode(ul);
    }
  });
});

$("#btn-note-export").addEventListener("click", async () => {
  if (!state.currentPage) { alert("请先选择页面"); return; }
  try {
    const path = await invoke<string>("export_page", { pageId: state.currentPage });
    alert("✅ 已导出: " + path);
  } catch (e) { alert("导出失败: " + e); }
});

$("#btn-note-voice").addEventListener("click", async () => {
  try {
    const text = await invoke<string>("voice_record");
    ($("#note-body") as HTMLElement).textContent += (($("#note-body") as HTMLElement).textContent ? "\n" : "") + text;
  } catch (e) { alert("语音识别失败: " + e); }
});

$("#btn-note-attach").addEventListener("click", async () => {
  if (!state.currentPage) { alert("请先选择页面"); return; }
  try {
    const path = await invoke<string>("attach_file", { pageId: state.currentPage });
    const body = ($("#note-body") as HTMLElement);
    const lower = path.toLowerCase();
    if (/\.(png|jpe?g|gif|webp|bmp)$/.test(lower)) {
      body.innerHTML += `<br><img src="file://${path}" />`;
    } else {
      body.innerHTML += `<br><a href="file://${path}">📎 ${path.split(/[\\/]/).pop()}</a>`;
    }
  } catch (e) { alert("添加附件失败: " + e); }
});

// ============ 转换 ============
$("#btn-pick-file").addEventListener("click", async () => {
  try {
    const path = await invoke<string>("pick_file");
    if (path) {
      state.currentFile = path;
      $("#convert-file-name").textContent = path.split(/[\\/]/).pop() || path;
      await loadConvertTargets(path);
    }
  } catch (e) { alert("选择文件失败: " + e); }
});

async function loadConvertTargets(filePath: string) {
  const targets = await invoke<Array<{ ext: string; label: string }>>("convert_targets", { filePath });
  const sel = $("#convert-target") as HTMLSelectElement;
  sel.innerHTML = '<option value="">选择目标格式…</option>';
  targets.forEach((t) => {
    const opt = document.createElement("option");
    opt.value = t.ext;
    opt.textContent = t.label;
    sel.appendChild(opt);
  });
  ($("#btn-convert") as HTMLButtonElement).disabled = targets.length === 0;
}

$("#btn-convert").addEventListener("click", async () => {
  if (!state.currentFile) return;
  const target = ($("#convert-target") as HTMLSelectElement).value;
  if (!target) return;
  setStatus("#convert-status", "转换中…");
  try {
    const outPath = await invoke<string>("convert_file", { filePath: state.currentFile, targetExt: target });
    setStatus("#convert-status", `✅ 转换完成: ${outPath}`, "ok");
  } catch (e) {
    setStatus("#convert-status", `转换失败: ${e}`, "err");
  }
});

// ---------- 启动加载 ----------
loadNotebooks().catch((e) => console.error("笔记加载失败:", e));
