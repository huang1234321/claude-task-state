// Resolve invoke across Tauri v2 global shapes (core.invoke / tauri.invoke / legacy).
const invoke =
  window.__TAURI__?.core?.invoke ||
  window.__TAURI__?.tauri?.invoke ||
  window.__TAURI__?.invoke;

// Resolve the current window for drag / hide.
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

// Drag the borderless window by its background (not when grabbing a card or the close button).
document.getElementById("app").addEventListener("mousedown", (e) => {
  if (e.target.closest(".close-btn") || e.target.closest(".card")) return;
  getCurrentWin()?.startDragging?.();
});

// Close button → hide window to tray (use the tray "Quit" to exit fully).
document.querySelector(".close-btn")?.addEventListener("click", () => getCurrentWin()?.hide?.());

tick();
setInterval(tick, 1000);
