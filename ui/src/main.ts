// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The app shell: wires the page to the API. Every change to the document is
// an API request, so undo, redo and unsaved-change tracking come from the
// core, not from here.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import {
  ApiFailure,
  type DocumentState,
  execute,
  type Parameter,
  request,
  state,
  type StepInfo,
  stepPreview,
} from "./api";
import { Viewer } from "./viewer";

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`missing #${id}`);
  return found as T;
}

const ui = {
  docName: element<HTMLInputElement>("doc-name"),
  savedState: element<HTMLSpanElement>("saved-state"),
  undo: element<HTMLButtonElement>("undo"),
  redo: element<HTMLButtonElement>("redo"),
  parameterList: element<HTMLUListElement>("parameter-list"),
  noParameters: element<HTMLParagraphElement>("no-parameters"),
  parameterForm: element<HTMLFormElement>("parameter-form"),
  paramName: element<HTMLInputElement>("param-name"),
  paramValue: element<HTMLInputElement>("param-value"),
  paramUnit: element<HTMLSelectElement>("param-unit"),
  viewportHint: element<HTMLDivElement>("viewport-hint"),
  modelEmpty: element<HTMLDivElement>("model-empty"),
  modelInfo: element<HTMLDListElement>("model-info"),
  modelNote: element<HTMLParagraphElement>("model-note"),
  statusPath: element<HTMLSpanElement>("status-path"),
  statusUndo: element<HTMLSpanElement>("status-undo"),
  toast: element<HTMLDivElement>("toast"),
};

const viewer = new Viewer(element("viewport"));
const appWindow = getCurrentWindow();
let current: DocumentState | null = null;
let toastTimer: number | undefined;

function showError(error: unknown): void {
  const message =
    error instanceof ApiFailure || error instanceof Error ? error.message : String(error);
  ui.toast.textContent = message;
  ui.toast.hidden = false;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    ui.toast.hidden = true;
  }, 6000);
}

/** Runs an action and reports any failure, so no click fails silently. */
function guarded(action: () => Promise<void>): () => void {
  return () => {
    action().catch(showError);
  };
}

const unitSymbol: Record<Parameter["unit"], string> = {
  millimetre: " mm",
  degree: " deg",
  unitless: "",
};

function formatNumber(value: number): string {
  return Number.isInteger(value) ? String(value) : String(Number(value.toFixed(6)));
}

function render(next: DocumentState): void {
  current = next;
  const { document: doc, path, dirty, undo, redo } = next;

  if (globalThis.document.activeElement !== ui.docName) {
    ui.docName.value = doc.name;
  }
  ui.savedState.textContent = dirty ? "Unsaved changes" : "Saved";
  ui.savedState.classList.toggle("dirty", dirty);
  ui.undo.disabled = undo === null;
  ui.undo.title = undo ? `Undo ${undo}` : "Nothing to undo";
  ui.redo.disabled = redo === null;
  ui.redo.title = redo ? `Redo ${redo}` : "Nothing to redo";
  ui.statusPath.textContent = path ?? "Not saved yet";
  ui.statusUndo.textContent = undo ? `Last change: ${undo}` : "";

  ui.parameterList.replaceChildren();
  const entries = Object.entries(doc.parameters);
  ui.noParameters.hidden = entries.length > 0;
  for (const [name, parameter] of entries) {
    const item = globalThis.document.createElement("li");
    const label = globalThis.document.createElement("span");
    label.className = "name";
    label.textContent = name;
    label.title = parameter.comment ?? name;
    const value = globalThis.document.createElement("span");
    value.className = "value";
    value.textContent = `${formatNumber(parameter.value)}${unitSymbol[parameter.unit]}`;
    const remove = globalThis.document.createElement("button");
    remove.type = "button";
    remove.setAttribute("aria-label", `Remove ${name}`);
    remove.innerHTML =
      '<svg aria-hidden="true" viewBox="0 0 20 20"><path d="M5 5l10 10M15 5L5 15" /></svg>';
    remove.addEventListener(
      "click",
      guarded(async () => render(await execute("parameter.remove", { name }))),
    );
    item.addEventListener("dblclick", () => {
      ui.paramName.value = name;
      ui.paramValue.value = String(parameter.value);
      ui.paramUnit.value =
        parameter.unit === "millimetre" ? "mm" : parameter.unit === "degree" ? "deg" : "none";
      ui.paramValue.focus();
    });
    item.append(label, value, remove);
    ui.parameterList.append(item);
  }

  const title = `${dirty ? "• " : ""}${doc.name} — CNDT`;
  appWindow.setTitle(title).catch(() => {
    // The title is cosmetic; never interrupt work over it.
  });
}

/** Asks before throwing away unsaved changes. Returns true to go ahead. */
async function confirmDiscard(action: string): Promise<boolean> {
  if (!current?.dirty) return true;
  return ask(`The document has unsaved changes. ${action} anyway and lose them?`, {
    title: "Unsaved changes",
    kind: "warning",
    okLabel: "Discard changes",
    cancelLabel: "Cancel",
  });
}

async function saveAs(): Promise<boolean> {
  const path = await save({
    title: "Save project",
    defaultPath: `${current?.document.name ?? "Untitled"}.cndt`,
    filters: [{ name: "CNDT project", extensions: ["cndt"] }],
  });
  if (!path) return false;
  render(await request<DocumentState>({ method: "save_as", path }));
  return true;
}

async function saveCurrent(): Promise<boolean> {
  if (!current?.path) return saveAs();
  render(await request<DocumentState>({ method: "save" }));
  return true;
}

function showModel(info: StepInfo, path: string): void {
  const rows: [string, string][] = [
    ["File", path.split(/[\\/]/).pop() ?? path],
    ["Solids", String(info.solids)],
    ["Faces", String(info.faces)],
    ["Edges", String(info.edges)],
    ["Vertices", String(info.vertices)],
  ];
  if (info.bounds) {
    const { min, max } = info.bounds;
    const size = [max.x - min.x, max.y - min.y, max.z - min.z].map((v) => v.toFixed(2));
    rows.push(["Size", `${size.join(" × ")} mm`]);
  }
  ui.modelInfo.replaceChildren(
    ...rows.flatMap(([term, detail]) => {
      const dt = globalThis.document.createElement("dt");
      dt.textContent = term;
      const dd = globalThis.document.createElement("dd");
      dd.textContent = detail;
      return [dt, dd];
    }),
  );
  ui.modelInfo.hidden = false;
  ui.modelNote.hidden = false;
  ui.modelEmpty.hidden = true;
  ui.viewportHint.hidden = true;
}

element("new").addEventListener(
  "click",
  guarded(async () => {
    if (!(await confirmDiscard("Start a new document"))) return;
    render(
      await request<DocumentState>({
        method: "new_document",
        name: "Untitled",
        discard_changes: true,
      }),
    );
  }),
);

element("open").addEventListener(
  "click",
  guarded(async () => {
    const path = await open({
      title: "Open project",
      multiple: false,
      directory: false,
      filters: [{ name: "CNDT project", extensions: ["cndt"] }],
    });
    if (!path) return;
    if (!(await confirmDiscard("Open another project"))) return;
    render(await request<DocumentState>({ method: "open", path, discard_changes: true }));
  }),
);

element("save").addEventListener(
  "click",
  guarded(async () => {
    await saveCurrent();
  }),
);
element("save-as").addEventListener(
  "click",
  guarded(async () => {
    await saveAs();
  }),
);

element("import-step").addEventListener(
  "click",
  guarded(async () => {
    const path = await open({
      title: "Import STEP",
      multiple: false,
      directory: false,
      filters: [{ name: "STEP", extensions: ["step", "stp", "STEP", "STP"] }],
    });
    if (!path) return;
    ui.viewportHint.textContent = "Reading the file…";
    ui.viewportHint.hidden = false;
    try {
      const info = await request<StepInfo>({ method: "step_info", path });
      viewer.show(await stepPreview(path));
      showModel(info, path);
    } catch (error) {
      ui.viewportHint.textContent = "Import a STEP file to see it here.";
      throw error;
    }
  }),
);

element("fit").addEventListener("click", () => viewer.fit());

ui.undo.addEventListener(
  "click",
  guarded(async () => render(await request<DocumentState>({ method: "undo" }))),
);
ui.redo.addEventListener(
  "click",
  guarded(async () => render(await request<DocumentState>({ method: "redo" }))),
);

ui.docName.addEventListener(
  "change",
  guarded(async () => {
    const name = ui.docName.value.trim();
    if (!current || name === current.document.name) return;
    try {
      render(await execute("document.rename", { name }));
    } catch (error) {
      ui.docName.value = current.document.name;
      throw error;
    }
  }),
);
ui.docName.addEventListener("keydown", (event) => {
  if (event.key === "Enter") ui.docName.blur();
});

ui.parameterForm.addEventListener("submit", (event) => {
  event.preventDefault();
  guarded(async () => {
    render(
      await execute("parameter.set", {
        name: ui.paramName.value.trim(),
        value: Number(ui.paramValue.value),
        unit: ui.paramUnit.value,
      }),
    );
    ui.parameterForm.reset();
    ui.paramName.focus();
  })();
});

// Keyboard shortcuts. Text fields keep their own undo.
window.addEventListener("keydown", (event) => {
  const target = event.target as HTMLElement | null;
  if (target && (target.tagName === "INPUT" || target.tagName === "SELECT")) return;
  const mod = event.metaKey || event.ctrlKey;
  if (!mod) return;
  const key = event.key.toLowerCase();
  if (key === "z" && !event.shiftKey) {
    event.preventDefault();
    ui.undo.click();
  } else if ((key === "z" && event.shiftKey) || key === "y") {
    event.preventDefault();
    ui.redo.click();
  } else if (key === "s") {
    event.preventDefault();
    guarded(async () => {
      if (event.shiftKey) await saveAs();
      else await saveCurrent();
    })();
  }
});

// Never lose work by closing the window.
appWindow
  .onCloseRequested(async (event) => {
    if (!(await confirmDiscard("Close"))) event.preventDefault();
  })
  .catch(showError);

state().then(render).catch(showError);
