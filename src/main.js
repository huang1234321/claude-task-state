// Resolve invoke across Tauri v2 global shapes (core.invoke / tauri.invoke / legacy).
const invoke =
  window.__TAURI__?.core?.invoke ||
  window.__TAURI__?.tauri?.invoke ||
  window.__TAURI__?.invoke;

// Tiny on-screen diagnostics (temp) so drag/close failures aren't silent.
function dbg(msg) {
  const el = document.getElementById("dbg");
  if (el) el.textContent = msg;
  console.log("[dbg]", msg);
}

// Canonical Tauri v2 (withGlobalTauri): window.__TAURI__.webviewWindow.getCurrentWebviewWindow()
function getCurrentWin() {
  const T = window.__TAURI__ || {};
  const fn = T.webviewWindow?.getCurrentWebviewWindow || T.window?.getCurrentWindow;
  return fn ? fn() : null;
}

const STATE_LABEL = {
  starting: "…",
  working: "working",
  waiting: "waiting",
  waitingpermission: "your move",
  error: "error",
  ended: "ended",
};

function escapeHtml(t) {
  return t.replace(/[&<>"']/g, (c) => ({ "&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;" }[c]));
}

function render(sessions) {
  const list = document.getElementById("list");
  if (!sessions || !sessions.length) {
    list.innerHTML = '<div class="empty">No active Claude Code sessions</div>';
    return;
  }
  list.innerHTML = sessions
    .map((s) => `
      <div class="card">
        <span class="dot ${s.color}"></span>
        <span class="label">${escapeHtml(s.project)}</span>
        <span class="state">${STATE_LABEL[s.state] || s.state}</span>
      </div>`)
    .join("");
}

async function tick() {
  if (!invoke) return;
  try { render(await invoke("get_sessions")); } catch (e) { console.error(e); }
}

// Probe what's actually available (helps diagnose if drag/close still fail).
(function probe() {
  const T = window.__TAURI__;
  dbg(T ? "TAURI ns: " + Object.keys(T).join(",") : "no __TAURI__");
})();

// Drag the borderless window by its background (not on cards / close button).
document.getElementById("app").addEventListener("mousedown", (e) => {
  if (e.target.closest(".close-btn") || e.target.closest(".card")) return;
  const w = getCurrentWin();
  if (!w || !w.startDragging) { dbg("drag: no startDragging fn"); return; }
  dbg("drag: calling startDragging");
  try {
    const p = w.startDragging();
    if (p && p.catch) p.catch((err) => dbg("drag ERR: " + err));
  } catch (err) { dbg("drag throw: " + err); }
});

// Close button → hide window to tray (tray Quit exits fully).
document.querySelector(".close-btn")?.addEventListener("click", () => {
  const w = getCurrentWin();
  if (!w || !w.hide) { dbg("close: no hide fn"); return; }
  dbg("close: calling hide");
  try {
    const p = w.hide();
    if (p && p.catch) p.catch((err) => dbg("close ERR: " + err));
  } catch (err) { dbg("close throw: " + err); }
});

tick();
setInterval(tick, 1000);
