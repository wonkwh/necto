//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import "./style.css";

import { createDetailPane, isNectoBridgeError, necto } from "@necto/bridge";
import { t } from "./localization";

type Direction = "out" | "in" | "state";

interface Row {
  id: string;
  seq: number;
  at: number;
  channel: string;
  direction: Direction;
  preview: string;
  byteCount: number;
  isBinary: boolean;
}

interface Detail extends Row {
  text: string;
}

/// Per channel, matching the app: a chatty channel does not push a quiet one out.
const maximumRowCount = 500;
const maximumBodyLength = 20_000;
const arrows: Record<Direction, string> = { out: "↑", in: "↓", state: "•" };
const tones: Record<Direction, string> = { out: "info", in: "ok", state: "idle" };

// -- state -------------------------------------------------------------------

const rows = new Map<string, Row>();
let channel = "";
let query = "";
let selectedID: string | null = null;
let detail: Detail | null = null;
let message: string | null = null;
const detailPane = createDetailPane("necto.realtime.detail");

/// What the app sent is never put in innerHTML; it becomes text nodes.
function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attributes: Record<string, string | undefined> = {},
  children: (Node | string)[] = [],
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attributes)) {
    if (value !== undefined) node.setAttribute(name, value);
  }
  node.append(...children);
  return node;
}

function messageOf(error: unknown): string {
  return isNectoBridgeError(error) ? error.message : String(error);
}

/// Read by position, so not in a locale's format.
function formatTime(milliseconds: number): string {
  const date = new Date(milliseconds);
  const pad = (value: number, width = 2) => String(value).padStart(width, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}.${pad(date.getMilliseconds(), 3)}`;
}

function formatSize(bytes: number): string {
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} kB`;
  return `${bytes} B`;
}

function matches(row: Row): boolean {
  if (channel && row.channel !== channel) return false;
  if (!query) return true;
  const needle = query.toLowerCase();
  return row.channel.toLowerCase().includes(needle) || row.preview.toLowerCase().includes(needle);
}

function visible(): Row[] {
  return [...rows.values()].filter(matches).sort((a, b) => b.seq - a.seq);
}

/// Returns the ids it dropped to stay within the per-channel limit.
function upsert(row: Row): string[] {
  rows.set(row.id, row);
  const sameChannel = [...rows.values()].filter((other) => other.channel === row.channel);
  if (sameChannel.length <= maximumRowCount) return [];
  const oldest = sameChannel.sort((a, b) => a.seq - b.seq).slice(0, sameChannel.length - maximumRowCount);
  for (const old of oldest) rows.delete(old.id);
  return oldest.map((old) => old.id);
}

// -- the shell ---------------------------------------------------------------

const search = el("input", {
  class: "necto-field realtime-search",
  type: "search",
  placeholder: t("Search channel or message"),
  "aria-label": t("Search"),
});
const channelFilter = el("select", { class: "necto-field realtime-channel", "aria-label": t("Filter by channel") });
const count = el("span", { class: "necto-badge" }, ["0"]);
const clearButton = el("button", { type: "button", class: "necto-button necto-button-quiet necto-button-danger" }, [t("Clear")]);
const body = el("div", { class: "necto-body" });
const app = el("div", { class: "necto-app" }, [
  el("div", { class: "necto-toolbar" }, [search, channelFilter, count, el("span", { style: "margin-left:auto" }), clearButton]),
  body,
]);
document.getElementById("root")?.append(app);

// -- rendering ---------------------------------------------------------------

function direction(row: Row): HTMLSpanElement {
  return el("span", { class: `necto-status necto-status-${tones[row.direction]}` }, [arrows[row.direction]]);
}

function line(row: Row): HTMLTableRowElement {
  const node = el("tr", { class: "realtime-row", tabindex: "0", "aria-selected": row.id === selectedID ? "true" : undefined }, [
    el("td", { class: "necto-numeric" }, [formatTime(row.at)]),
    el("td", {}, [el("span", { class: "necto-badge realtime-cell", title: row.channel }, [row.channel])]),
    el("td", {}, [direction(row)]),
    el("td", {}, [el("span", { class: "realtime-cell realtime-message" }, [row.preview])]),
  ]);
  node.addEventListener("click", () => void open(row.id));
  node.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      void open(row.id);
    }
  });
  return node;
}

let channelSignature = "";

function updateChannelOptions(): void {
  const channels = [...new Set([...rows.values()].map((row) => row.channel))].sort();
  const signature = channels.join("\u001f");
  if (signature === channelSignature) return;
  channelSignature = signature;
  channelFilter.replaceChildren(
    el("option", { value: "" }, [t("All channels")]),
    ...channels.map((value) => el("option", { value }, [value])),
  );
  if (!channels.includes(channel)) channel = "";
  channelFilter.value = channel;
}

function render(): void {
  updateChannelOptions();
  const shown = visible();
  if (selectedID && !shown.some((row) => row.id === selectedID)) {
    selectedID = null;
    detail = null;
    showDetail();
  }
  count.textContent = shown.length === rows.size ? String(shown.length) : `${shown.length} / ${rows.size}`;

  if (!shown.length) {
    body.replaceChildren(
      el("div", { class: "necto-empty" }, [
        el("p", { class: "necto-empty-title" }, [message ?? t("Nothing captured yet")]),
        el("p", { class: "necto-caption" }, [t("Socket, Bluetooth and stream messages of the connected app appear here as they happen.")]),
      ]),
    );
    return;
  }

  body.replaceChildren(
    el("table", { class: "necto-table realtime-table" }, [
      el("thead", {}, [
        el("tr", {}, [
          el("th", { class: "realtime-col-time necto-numeric" }, [t("Time")]),
          el("th", { class: "realtime-col-channel" }, [t("Channel")]),
          el("th", { class: "realtime-col-direction" }, [""]),
          el("th", {}, [t("Message")]),
        ]),
      ]),
      el("tbody", {}, shown.map(line)),
    ]),
  );
}

async function open(id: string): Promise<void> {
  selectedID = selectedID === id ? null : id;
  detail = null;
  render();
  showDetail();
  if (!selectedID) return;

  const requestedID = selectedID;
  try {
    // Only the selected row's full text is fetched.
    const output = await necto.device.send<{ record: Detail }>("realtime.detail", { id: requestedID });
    if (selectedID !== requestedID) return;
    detail = output.record;
  } catch (error) {
    if (selectedID !== requestedID) return;
    const row = rows.get(requestedID);
    if (row) detail = { ...row, text: messageOf(error) };
  }
  showDetail();
}

/// A long body draws its head; the rest is one click away.
function code(text: string): Node[] {
  const output = el("pre", { class: "necto-code realtime-code" }, [text.slice(0, maximumBodyLength)]);
  if (text.length <= maximumBodyLength) return [output];
  const expand = el("button", { type: "button", class: "necto-button necto-button-quiet" }, [t("Show full value")]);
  expand.addEventListener("click", () => {
    output.textContent = text;
    expand.remove();
  });
  return [output, expand];
}

function bodySection(text: string): Node {
  let shown = text;
  try {
    shown = JSON.stringify(JSON.parse(text), null, 2);
  } catch {}
  return el("section", {}, [el("h3", { class: "necto-section-title" }, [t("Body")]), ...code(shown)]);
}

function showDetail(): void {
  detailPane.unmount();
  app.querySelector(".necto-detail")?.remove();
  const row = selectedID ? rows.get(selectedID) : null;
  if (!row) return;

  const close = el("button", { type: "button", class: "necto-button necto-button-quiet", "aria-label": t("Close details") }, ["✕"]);
  close.addEventListener("click", () => void open(row.id));
  const pane = el("aside", { class: "necto-detail" }, [
    el("div", { class: "necto-detail-title" }, [
      direction(row),
      el("span", { class: "necto-badge" }, [row.channel]),
      el("span", { class: "necto-numeric" }, [formatTime(row.at)]),
      el("span", { class: "necto-detail-title-url" }, [row.preview.slice(0, 200)]),
      close,
    ]),
    el("div", { class: "necto-detail-body realtime-detail" }, [
      el("p", { class: "necto-caption" }, [formatSize(row.byteCount)]),
      detail === null ? el("p", { class: "necto-caption" }, [t("Loading…")]) : bodySection(detail.text),
    ]),
  ]);
  app.append(pane);
  detailPane.mount(pane);
}

// -- actions -----------------------------------------------------------------

search.addEventListener("input", () => {
  query = search.value;
  render();
});

channelFilter.addEventListener("change", () => {
  channel = channelFilter.value;
  render();
});

clearButton.addEventListener("click", () => {
  necto.device
    .send("realtime.clear")
    .then(() => {
      rows.clear();
      selectedID = null;
      detail = null;
      render();
    })
    .catch((error: unknown) => {
      message = messageOf(error);
      render();
    });
});

// -- start -------------------------------------------------------------------

let renderScheduled = false;

/// Draws at most once a frame, however fast messages arrive.
function scheduleRender(): void {
  if (renderScheduled) return;
  renderScheduled = true;
  requestAnimationFrame(() => {
    renderScheduled = false;
    render();
  });
}

async function main(): Promise<void> {
  if (!necto.isAvailable()) {
    message = t("This plugin only works inside Necto");
    render();
    return;
  }
  render();

  try {
    // What the app collected since it started, then what arrives next.
    const page = await necto.device.send<{ records: Row[] }>("realtime.list", { limit: maximumRowCount });
    for (const row of page.records) upsert(row);
    render();

    await necto.device.subscribe<{ record: Row }>(
      "realtime.observe",
      {},
      ({ record }) => {
        const removed = upsert(record);
        if (selectedID && removed.includes(selectedID)) {
          selectedID = null;
          detail = null;
        }
        scheduleRender();
      },
      (error) => {
        message = error.message;
        render();
      },
    );
  } catch (error) {
    message = messageOf(error);
    render();
  }

  // Ready only once every handler is in place; the host holds events until then.
  await necto.ready();
}

void main();
