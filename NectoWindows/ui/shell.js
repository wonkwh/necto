//
//  Copyright (c) 2026 Viva Republica, Inc.
//

// The shell relays between sandboxed panel iframes and the Rust host. A request's
// sender is known only by its iframe window, never by anything the panel says.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// WebView2 serves custom schemes as http://<scheme>.localhost; WebKit keeps the scheme.
const base = navigator.userAgent.includes("Windows") ? "http://necto-plugin.localhost" : "necto-plugin://localhost";

const sidebar = document.getElementById("sidebar");
const panes = document.getElementById("panes");
const empty = document.getElementById("empty");

/** iframe by "handle/plugin", and the reverse lookup from its window. */
const frames = new Map();
const owners = new Map();
let selected = null;

const theme = document.createElement("link");
theme.rel = "stylesheet";
theme.href = `${base}/theme.css`;
document.head.prepend(theme);

function keyOf(handle, plugin) {
  return `${handle}/${plugin}`;
}

function select(handle, plugin) {
  selected = keyOf(handle, plugin);
  if (!frames.has(selected)) {
    const frame = document.createElement("iframe");
    frame.sandbox = "allow-scripts";
    frame.title = plugin;
    frame.src = `${base}/${encodeURIComponent(handle)}/${encodeURIComponent(plugin)}/index.html`;
    panes.append(frame);
    frames.set(selected, frame);
    owners.set(frame.contentWindow, { handle, plugin });
  }
  for (const [key, frame] of frames) frame.hidden = key !== selected;
  for (const button of sidebar.querySelectorAll(".plugin")) {
    button.setAttribute("aria-current", String(button.dataset.key === selected));
  }
}

function render(apps) {
  sidebar.replaceChildren();
  for (const app of apps) {
    const header = document.createElement("div");
    header.className = "app";
    if (app.appIcon) {
      const icon = document.createElement("img");
      icon.src = `data:image/png;base64,${app.appIcon}`;
      icon.alt = "";
      header.append(icon);
    }
    const label = document.createElement("div");
    const name = document.createElement("strong");
    name.textContent = app.appName;
    const device = document.createElement("small");
    device.textContent = app.osVersion ? `${app.deviceName} · Android ${app.osVersion}` : app.deviceName;
    label.append(name, device);
    header.append(label);
    sidebar.append(header);

    if (app.plugins.length === 0) {
      const waiting = document.createElement("div");
      waiting.className = "waiting";
      waiting.textContent = "Loading panels…";
      sidebar.append(waiting);
    }
    for (const plugin of app.plugins) {
      const button = document.createElement("button");
      button.className = "plugin";
      button.textContent = plugin.name;
      button.dataset.key = keyOf(app.handle, plugin.id);
      button.addEventListener("click", () => select(app.handle, plugin.id));
      sidebar.append(button);
    }
  }
}

async function refresh() {
  const apps = await invoke("targets");
  render(apps);

  const live = new Set(apps.flatMap((app) => app.plugins.map((plugin) => keyOf(app.handle, plugin.id))));
  for (const [key, frame] of frames) {
    if (live.has(key)) continue;
    owners.delete(frame.contentWindow);
    frame.remove();
    frames.delete(key);
  }
  empty.hidden = live.size > 0;
  if (selected && live.has(selected)) {
    const [handle, plugin] = selected.split("/");
    select(handle, plugin);
  } else {
    const first = apps.find((app) => app.plugins.length > 0);
    selected = null;
    if (first) select(first.handle, first.plugins[0].id);
  }
}

window.addEventListener("message", async (event) => {
  const owner = owners.get(event.source);
  const id = event.data?.nectoRequest;
  if (!owner || !id) return;
  const reply = await invoke("bridge", { handle: owner.handle, plugin: owner.plugin, message: event.data.message });
  event.source.postMessage({ nectoReply: id, reply }, "*");
});

listen("necto://stream", ({ payload }) => {
  const frame = frames.get(keyOf(payload.handle, payload.plugin));
  frame?.contentWindow.postMessage(
    { nectoStream: payload.kind, subscriptionID: payload.subscriptionID, payload: payload.payload },
    "*",
  );
});

listen("necto://targets", refresh);
refresh();
