// Resolve invoke across Tauri v2 global shapes (core.invoke / tauri.invoke / legacy).
const invoke =
  window.__TAURI__?.core?.invoke ||
  window.__TAURI__?.tauri?.invoke ||
  window.__TAURI__?.invoke;

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

tick();
setInterval(tick, 1000);
