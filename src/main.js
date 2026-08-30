// Resolve invoke across Tauri v2 global shapes (core.invoke / tauri.invoke / legacy).
const invoke =
  window.__TAURI__?.core?.invoke ||
  window.__TAURI__?.tauri?.invoke ||
  window.__TAURI__?.invoke;

// Canonical Tauri v2 (withGlobalTauri): window.__TAURI__.webviewWindow.getCurrentWebviewWindow()
function getCurrentWin() {
  const T = window.__TAURI__ || {};
  const fn = T.webviewWindow?.getCurrentWebviewWindow || T.window?.getCurrentWindow;
  return fn ? fn() : null;
}

const STATE_LABEL = {
  starting: "starting",
  working: "working",
  waiting: "your move",
  waitingpermission: "approve?",
  error: "error",
  ended: "ended",
};

// Card order: what needs the user first, autonomous work below, dead last.
const STATE_RANK = {
  error: 0,
  waitingpermission: 1,
  waiting: 2,
  working: 3,
  starting: 4,
  ended: 5,
};

const list = document.getElementById("list");
const prevStates = new Map(); // session_id -> state at last render

function stateLabel(s) {
  return STATE_LABEL[s.state] || s.state;
}

function makeCard(s) {
  const el = document.createElement("div");
  el.className = "card enter";
  el.dataset.id = s.session_id;
  el.setAttribute("role", "listitem");
  const dot = document.createElement("span");
  dot.className = "dot";
  const label = document.createElement("span");
  label.className = "label";
  const state = document.createElement("span");
  state.className = "state";
  el.append(dot, label, state);
  el.addEventListener("animationend", () => el.classList.remove("enter", "flash"));
  return el;
}

function updateCard(el, s) {
  el.dataset.color = s.color;
  el.dataset.state = s.state;
  el.children[1].textContent = s.project;
  el.children[2].textContent = stateLabel(s);
  el.title = s.project + " — " + stateLabel(s);
  if (prevStates.get(s.session_id) !== undefined && prevStates.get(s.session_id) !== s.state) {
    el.classList.remove("flash");
    void el.offsetWidth; // restart the animation
    el.classList.add("flash");
  }
  prevStates.set(s.session_id, s.state);
}

function makeEmpty() {
  const el = document.createElement("div");
  el.className = "empty";
  const l1 = document.createElement("div");
  l1.textContent = "No active Claude Code sessions";
  const l2 = document.createElement("div");
  l2.className = "sub";
  l2.textContent = "waiting for heartbeats…";
  el.append(l1, l2);
  return el;
}

// Keyed update: cards persist across ticks (no innerHTML rewrite, no flicker),
// so CSS transitions and change flashes only fire on real changes.
function render(sessions) {
  if (!sessions || !sessions.length) {
    list.replaceChildren(makeEmpty());
    prevStates.clear();
    return;
  }
  list.querySelector(".empty")?.remove();
  const ordered = [...sessions].sort(
    (a, b) =>
      (STATE_RANK[a.state] ?? 9) - (STATE_RANK[b.state] ?? 9) ||
      a.project.localeCompare(b.project)
  );
  const cards = new Map();
  for (const el of list.querySelectorAll(".card")) cards.set(el.dataset.id, el);
  const seen = new Set();
  for (const s of ordered) {
    seen.add(s.session_id);
    if (!cards.has(s.session_id)) cards.set(s.session_id, makeCard(s));
    updateCard(cards.get(s.session_id), s);
  }
  for (const [id, el] of cards) {
    if (!seen.has(id)) {
      el.remove();
      prevStates.delete(id);
    }
  }
  list.append(...ordered.map((s) => cards.get(s.session_id)));
}

async function tick() {
  if (!invoke) return;
  try {
    render(await invoke("get_sessions"));
  } catch (e) {
    console.error("get_sessions failed:", e);
  }
}

// Drag the borderless window by its background (not on cards / close button).
document.getElementById("app").addEventListener("mousedown", (e) => {
  if (e.target.closest(".close-btn") || e.target.closest(".card")) return;
  const w = getCurrentWin();
  if (!w?.startDragging) return;
  try {
    w.startDragging()?.catch?.((err) => console.error("drag failed:", err));
  } catch (err) {
    console.error("drag failed:", err);
  }
});

// Close button → fully quit the app (no lingering background process).
document.querySelector(".close-btn")?.addEventListener("click", async () => {
  try {
    await invoke("quit_app");
  } catch (err) {
    console.error("quit failed:", err);
  }
});

tick();
setInterval(tick, 1000);
