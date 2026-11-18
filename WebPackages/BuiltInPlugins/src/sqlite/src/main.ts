//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import "./style.css";

import { isNectoBridgeError, necto } from "@necto/bridge";
import { t } from "./localization";

interface Database {
  path: string;
  name: string;
  size: number;
  modifiedAt: number;
}

interface Table {
  name: string;
  kind: "table" | "view";
  sql: string;
  rowCount?: number;
}

interface Result {
  columns: string[];
  rows: unknown[][];
  truncated: boolean;
  elapsedMs: number;
}

const storageKey = "necto.sqlite.database";

// -- state -------------------------------------------------------------------

let databases: Database[] = [];
let database = "";
let tables: Table[] = [];
let table = "";
let result: Result | null = null;
let message: string | null = null;
let running = false;

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

function formatSize(bytes: number): string {
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} kB`;
  return `${bytes} B`;
}

function quote(identifier: string): string {
  return `"${identifier.replaceAll('"', '""')}"`;
}

function remember(path: string): void {
  try {
    localStorage.setItem(storageKey, path);
  } catch {}
}

function remembered(): string {
  try {
    return localStorage.getItem(storageKey) ?? "";
  } catch {
    return "";
  }
}

// -- the shell ---------------------------------------------------------------

const databaseSelect = el("select", { class: "necto-field sqlite-database", "aria-label": t("Database") });
const refreshButton = el("button", { type: "button", class: "necto-button necto-button-quiet" }, [t("Refresh")]);
const databaseInfo = el("span", { class: "necto-caption" });
const tableList = el("nav", { class: "necto-tree", "aria-label": t("Tables") });
const sqlInput = el("textarea", {
  class: "necto-field sqlite-sql",
  spellcheck: "false",
  placeholder: t("SELECT * FROM …  (⌘↩ to run)"),
  "aria-label": "SQL",
});
const runButton = el("button", { type: "button", class: "necto-button necto-button-primary" }, [t("Run")]);
const limitSelect = el(
  "select",
  { class: "necto-field sqlite-limit", "aria-label": t("Row limit") },
  [200, 1000, 5000].map((count) => el("option", { value: String(count) }, [t("{count} rows", { count: count.toLocaleString() })])),
);
const status = el("span", { class: "necto-caption" });
const resultView = el("div", { class: "sqlite-result" });

document.getElementById("root")?.append(
  el("div", { class: "necto-app" }, [
    el("div", { class: "necto-toolbar" }, [databaseSelect, refreshButton, databaseInfo]),
    el("div", { class: "necto-body sqlite-main" }, [
      el("div", { class: "sqlite-tables" }, [tableList]),
      el("section", { class: "sqlite-work" }, [
        el("div", { class: "sqlite-editor" }, [sqlInput, el("div", { class: "sqlite-actions" }, [runButton, limitSelect, status])]),
        resultView,
      ]),
    ]),
  ]),
);

// -- rendering ---------------------------------------------------------------

function renderDatabases(): void {
  databaseSelect.replaceChildren(
    ...(databases.length
      ? databases.map((entry) => el("option", { value: entry.path }, [entry.path]))
      : [el("option", { value: "" }, [t("No SQLite files in the app sandbox")])]),
  );
  databaseSelect.value = database;
  const current = databases.find((entry) => entry.path === database);
  databaseInfo.textContent = current
    ? t("{size} · modified {time}", { size: formatSize(current.size), time: new Date(current.modifiedAt).toLocaleString() })
    : "";
}

function renderTables(): void {
  tableList.replaceChildren(
    ...tables.map((entry) => {
      const meta = entry.kind === "view" ? t("view") : (entry.rowCount?.toLocaleString() ?? "");
      const row = el(
        "button",
        { type: "button", class: "necto-tree-row", title: entry.sql, "aria-selected": String(entry.name === table) },
        [el("span", { class: "necto-tree-twist" }), el("span", { class: "necto-tree-name" }, [entry.name]), el("span", { class: "necto-tree-meta" }, [meta])],
      );
      row.addEventListener("click", () => {
        table = entry.name;
        sqlInput.value = `SELECT * FROM ${quote(entry.name)}`;
        renderTables();
        void run();
      });
      return row;
    }),
  );
}

function cell(value: unknown): HTMLTableCellElement {
  if (value === null) return el("td", { class: "sqlite-null" }, ["NULL"]);
  const text = String(value);
  return el("td", { title: text.length > 40 ? text : undefined }, [text]);
}

function renderResult(): void {
  runButton.disabled = running || !database;
  if (running) {
    status.textContent = t("Running…");
  } else if (result) {
    const values = { count: result.rows.length.toLocaleString(), ms: result.elapsedMs };
    status.textContent = result.truncated ? t("{count} rows (limit reached) · {ms} ms", values) : t("{count} rows · {ms} ms", values);
  } else {
    status.textContent = "";
  }

  if (message) {
    resultView.replaceChildren(el("div", { class: "necto-notice necto-notice-danger sqlite-error" }, [message]));
    return;
  }
  if (!result) {
    resultView.replaceChildren(
      el("div", { class: "necto-empty" }, [
        el("p", { class: "necto-empty-title" }, [database ? t("Pick a table or run a query") : t("No database selected")]),
        el("p", { class: "necto-caption" }, [t("Read-only — statements that write are refused.")]),
      ]),
    );
    return;
  }
  resultView.replaceChildren(
    el("table", { class: "necto-table" }, [
      el("thead", {}, [el("tr", {}, result.columns.map((name) => el("th", {}, [name])))]),
      el("tbody", {}, result.rows.map((row) => el("tr", {}, row.map(cell)))),
    ]),
  );
}

// -- actions -----------------------------------------------------------------

async function loadDatabases(): Promise<void> {
  try {
    ({ databases } = await necto.device.send<{ databases: Database[] }>("sqlite.databases"));
    const preferred = database || remembered();
    database = databases.some((entry) => entry.path === preferred) ? preferred : (databases[0]?.path ?? "");
    message = null;
  } catch (error) {
    databases = [];
    database = "";
    message = messageOf(error);
  }
  renderDatabases();
  await loadTables();
}

async function loadTables(): Promise<void> {
  tables = [];
  if (database) {
    try {
      ({ tables } = await necto.device.send<{ tables: Table[] }>("sqlite.tables", { database }));
    } catch (error) {
      message = messageOf(error);
    }
  }
  if (!tables.some((entry) => entry.name === table)) table = "";
  renderTables();
  renderResult();
}

async function run(): Promise<void> {
  const sql = sqlInput.value.trim();
  if (!database || !sql || running) return;
  running = true;
  message = null;
  renderResult();
  try {
    result = await necto.device.send<Result>("sqlite.query", { database, sql, limit: Number(limitSelect.value) });
  } catch (error) {
    result = null;
    message = messageOf(error);
  }
  running = false;
  renderResult();
}

databaseSelect.addEventListener("change", () => {
  database = databaseSelect.value;
  remember(database);
  table = "";
  result = null;
  message = null;
  renderDatabases();
  void loadTables();
});

refreshButton.addEventListener("click", () => void loadDatabases());
runButton.addEventListener("click", () => void run());
limitSelect.addEventListener("change", () => void run());
sqlInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
    event.preventDefault();
    void run();
  }
});

// -- start -------------------------------------------------------------------

async function main(): Promise<void> {
  if (!necto.isAvailable()) {
    message = t("This plugin only works inside Necto");
    renderResult();
    return;
  }
  renderResult();
  await loadDatabases();
  await necto.ready();
}

void main();
