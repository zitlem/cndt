// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The app shell: wires the page to the API. Every change to the document is
// an API request, so undo, redo and unsaved-change tracking come from the
// core, not from here.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import {
  ApiFailure,
  bodyEdges,
  bodyMesh,
  type DocumentState,
  planeLabel,
  toolpathPreview,
  type CamOperation,
  execute,
  type Parameter,
  request,
  state,
  type StepInfo,
  stepPreview,
} from "./api";
import { SketchEditor, type Tool } from "./sketch";
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
  statusSketch: element<HTMLSpanElement>("status-sketch"),
  sketchList: element<HTMLUListElement>("sketch-list"),
  noSketches: element<HTMLParagraphElement>("no-sketches"),
  newSketch: element<HTMLButtonElement>("new-sketch"),
  planeMenu: element<HTMLDivElement>("plane-menu"),
  sketchTools: element<HTMLDivElement>("sketch-tools"),
  featureList: element<HTMLOListElement>("feature-list"),
  noFeatures: element<HTMLParagraphElement>("no-features"),
  setupList: element<HTMLOListElement>("setup-list"),
  noSetups: element<HTMLParagraphElement>("no-setups"),
  extrudeButton: element<HTMLButtonElement>("extrude"),
  extrudeDialog: element<HTMLFormElement>("extrude-dialog"),
  extrudeTitle: element<HTMLHeadingElement>("extrude-title"),
  extrudeSketch: element<HTMLSelectElement>("extrude-sketch"),
  extrudeDistance: element<HTMLInputElement>("extrude-distance"),
  extrudeOperation: element<HTMLSelectElement>("extrude-operation"),
  holeButton: element<HTMLButtonElement>("hole"),
  holeDialog: element<HTMLFormElement>("hole-dialog"),
  holeTitle: element<HTMLHeadingElement>("hole-title"),
  holeSketch: element<HTMLSelectElement>("hole-sketch"),
  holeType: element<HTMLSelectElement>("hole-type"),
  holeDiameter: element<HTMLInputElement>("hole-diameter"),
  holeThread: element<HTMLSelectElement>("hole-thread"),
  holeCbDiameter: element<HTMLInputElement>("hole-cb-diameter"),
  holeCbDepth: element<HTMLInputElement>("hole-cb-depth"),
  holeDepth: element<HTMLInputElement>("hole-depth"),
  holeThrough: element<HTMLInputElement>("hole-through"),
  holeThreadDepth: element<HTMLInputElement>("hole-thread-depth"),
  holeTip: element<HTMLInputElement>("hole-tip"),
  holeFit: element<HTMLInputElement>("hole-fit"),
  holePoints: element<HTMLParagraphElement>("hole-points"),
  chamferButton: element<HTMLButtonElement>("chamfer"),
  patternButton: element<HTMLButtonElement>("pattern"),
  patternDialog: element<HTMLFormElement>("pattern-dialog"),
  patternTitle: element<HTMLHeadingElement>("pattern-title"),
  patternSource: element<HTMLSelectElement>("pattern-source"),
  patternCountX: element<HTMLInputElement>("pattern-count-x"),
  patternSpacingX: element<HTMLInputElement>("pattern-spacing-x"),
  patternCountY: element<HTMLInputElement>("pattern-count-y"),
  patternSpacingY: element<HTMLInputElement>("pattern-spacing-y"),
  filletButton: element<HTMLButtonElement>("fillet"),
  blendDialog: element<HTMLFormElement>("blend-dialog"),
  blendTitle: element<HTMLHeadingElement>("blend-title"),
  blendSizeLabel: element<HTMLSpanElement>("blend-size-label"),
  blendSize: element<HTMLInputElement>("blend-size"),
  blendPicked: element<HTMLParagraphElement>("blend-picked"),
  toast: element<HTMLDivElement>("toast"),
};

const viewer = new Viewer(element("viewport"));
const editor = new SketchEditor(element("viewport"), {
  async execute(command, args) {
    const next = await execute(command, args);
    render(next);
    return next;
  },
  fail(error) {
    showError(error);
    return error instanceof ApiFailure ? (error.details?.constraints ?? []) : [];
  },
  changed: () => updateSketchUi(),
});
const appWindow = getCurrentWindow();
let current: DocumentState | null = null;
let toastTimer: number | undefined;

/** Messages for failures the interface explains better than the raw text. */
function explain(error: unknown): string {
  if (error instanceof ApiFailure) {
    const named = (error.details?.constraints?.length ?? 0) > 0;
    if (error.kind === "conflict") {
      return named
        ? "That can't hold together with the constraints highlighted in red. Change or delete one of them first."
        : "The constraints can't all hold at once.";
    }
    if (error.kind === "redundant") {
      return named
        ? "Not needed: the constraints highlighted in red already fix that."
        : "Not needed: the existing constraints already fix that.";
    }
  }
  return error instanceof Error ? error.message : String(error);
}

function showError(error: unknown): void {
  const message = explain(error);
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

  renderSketches(next);
  renderFeatures(next);
  renderSetups(next);
  void showBody(next);
  editor.update(next);
  updateSketchUi();

  const title = `${dirty ? "• " : ""}${doc.name} — CNDT`;
  appWindow.setTitle(title).catch(() => {
    // The title is cosmetic; never interrupt work over it.
  });
}

const statusText: Record<string, string> = {
  fully: "Fully constrained",
  over: "Over-constrained",
  unknown: "Too large to analyse",
};

function freedomText(id: string, state: DocumentState): string {
  const freedom = state.freedom[id];
  if (!freedom) return "";
  const entities = state.document.sketches?.[id]?.sketch.entities ?? {};
  if (Object.keys(entities).length === 0) return "Empty";
  if (freedom.status === "under") {
    const n = freedom.dof ?? 0;
    return `${n} degree${n === 1 ? "" : "s"} of freedom`;
  }
  return statusText[freedom.status] ?? "";
}

function renderSketches(state: DocumentState): void {
  const sketches = Object.entries(state.document.sketches ?? {});
  ui.sketchList.replaceChildren();
  ui.noSketches.hidden = sketches.length > 0;
  for (const [id, record] of sketches) {
    const item = globalThis.document.createElement("li");
    item.classList.toggle("editing", editor.editing === Number(id));
    item.title = "Click to edit";
    const name = globalThis.document.createElement("span");
    name.className = "name";
    name.textContent = record.name;
    const badge = globalThis.document.createElement("span");
    const empty = Object.keys(record.sketch.entities).length === 0;
    const status = empty ? "under" : (state.freedom[id]?.status ?? "under");
    badge.className = `badge ${status}`;
    const frame = state.sketch_frames[id];
    badge.textContent = frame?.state === "failed" ? "⚠" : status === "fully" ? "✓" : planeLabel(record.plane);
    badge.title = frame?.state === "failed" ? frame.reason : freedomText(id, state);
    const remove = globalThis.document.createElement("button");
    remove.type = "button";
    remove.setAttribute("aria-label", `Delete ${record.name}`);
    remove.innerHTML =
      '<svg aria-hidden="true" viewBox="0 0 20 20"><path d="M5 5l10 10M15 5L5 15" /></svg>';
    remove.addEventListener("click", (event) => {
      event.stopPropagation();
      guarded(async () => {
        if (editor.editing === Number(id)) editor.close();
        render(await execute("sketch.delete", { sketch: Number(id) }));
      })();
    });
    item.addEventListener("click", () => {
      if (current) editSketch(Number(id), current);
      updateSketchUi();
    });
    item.append(name, badge, remove);
    ui.sketchList.append(item);
  }
  // As in Fusion 360, a sketch a feature has used is hidden in 3D.
  const used = new Set(
    (state.document.features ?? []).flatMap((f) => ("sketch" in f.feature ? [String(f.feature.sketch)] : [])),
  );
  const placed = sketches.flatMap(([id, record]) => {
    const status = state.sketch_frames[id];
    return !used.has(id) && status?.state === "ok" ? [{ record, frame: status.frame }] : [];
  });
  viewer.setSketches(placed);
}

let shownRevision = -1;
let bodyShown = false;
let bodyTriangles: { mesh: import("./api").Mesh } | null = null;

/** The triangles of the face a sketch is on, in that sketch's coordinates. */
function faceReference(id: number, state: DocumentState): import("./api").XY[] {
  const plane = state.document.sketches?.[String(id)]?.plane;
  const frame = state.sketch_frames[String(id)];
  if (!plane || typeof plane === "string" || frame?.state !== "ok" || !bodyTriangles || !state.body) return [];
  const face = state.body.faces.findIndex((names) => names.includes(plane.face));
  if (face < 0) return [];
  const { positions, indices, triangleFaces } = bodyTriangles.mesh;
  const { origin: o, x, y } = frame.frame;
  const out: import("./api").XY[] = [];
  triangleFaces.forEach((f, t) => {
    if (f !== face) return;
    for (let k = 0; k < 3; k++) {
      const v = indices[t * 3 + k] ?? 0;
      const px = (positions[v * 3] ?? 0) - o.x;
      const py = (positions[v * 3 + 1] ?? 0) - o.y;
      const pz = (positions[v * 3 + 2] ?? 0) - o.z;
      out.push({ x: px * x[0] + py * x[1] + pz * x[2], y: px * y[0] + py * y[1] + pz * y[2] });
    }
  });
  return out;
}

/** Opens a sketch for editing, with the face it is on shown under it. */
function editSketch(id: number, state: DocumentState): void {
  editor.open(id, state);
  editor.setReference(faceReference(id, state));
  editor.fit();
}

/** Fetches and shows the rebuilt part when it changed. */
async function showBody(state: DocumentState): Promise<void> {
  if (state.body_revision === shownRevision) return;
  shownRevision = state.body_revision;
  if (!state.body) {
    bodyTriangles = null;
    if (bodyShown) viewer.clear();
    bodyShown = false;
    renderBodyInfo(state);
    return;
  }
  try {
    const [mesh, edges] = await Promise.all([bodyMesh(), bodyEdges()]);
    bodyTriangles = { mesh };
    viewer.setEdges(edges);
    // Keep the camera where it is once a part is on screen.
    viewer.show(mesh, bodyShown);
    bodyShown = true;
    // A drawn toolpath follows the part.
    if (shownSetup !== null) await showToolpath(shownSetup).catch(() => showToolpath(null));
    renderBodyInfo(state);
  } catch (error) {
    showError(error);
  }
}

function renderBodyInfo(state: DocumentState): void {
  const body = state.body;
  if (!body) {
    if (!ui.modelNote.hidden) return; // a STEP preview is showing
    ui.modelInfo.hidden = true;
    ui.modelEmpty.hidden = false;
    return;
  }
  const { min, max } = body.bounds;
  const size = [max.x - min.x, max.y - min.y, max.z - min.z].map((v) => formatNumber(Number(v.toFixed(3))));
  const rows: [string, string][] = [
    ["Size", `${size.join(" × ")} mm`],
    ["Volume", `${formatNumber(Number((body.volume / 1000).toFixed(3)))} cm³`],
    ["Faces", String(body.faces.length)],
  ];
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
  ui.modelNote.hidden = true;
  ui.modelEmpty.hidden = true;
  ui.viewportHint.hidden = true;
}

function renderFeatures(state: DocumentState): void {
  const features = state.document.features ?? [];
  ui.featureList.replaceChildren();
  ui.noFeatures.hidden = features.length > 0;
  ui.extrudeButton.disabled = Object.keys(state.document.sketches ?? {}).length === 0;
  ui.holeButton.disabled = !state.body;
  ui.chamferButton.disabled = !state.body;
  ui.patternButton.disabled = !(state.document.features ?? []).some(
    (f) => f.feature.type === "extrude" || f.feature.type === "hole",
  );
  ui.filletButton.disabled = !state.body;
  for (const { id, feature } of features) {
    const status = state.feature_status[String(id)];
    const item = globalThis.document.createElement("li");
    const failed = status?.state === "failed";
    item.classList.toggle("failed", failed);
    item.title = failed ? status.reason : "Click to edit";
    const name = globalThis.document.createElement("span");
    name.className = "name";
    name.textContent = feature.name;
    const detail = globalThis.document.createElement("span");
    detail.className = failed ? "detail flag" : "detail";
    detail.textContent = failed ? "⚠ needs repair" : featureDetail(feature);
    const remove = globalThis.document.createElement("button");
    remove.type = "button";
    remove.setAttribute("aria-label", `Delete ${feature.name}`);
    remove.innerHTML =
      '<svg aria-hidden="true" viewBox="0 0 20 20"><path d="M5 5l10 10M15 5L5 15" /></svg>';
    remove.addEventListener("click", (event) => {
      event.stopPropagation();
      guarded(async () => render(await execute("feature.delete", { feature: id })))();
    });
    item.addEventListener("click", () => {
      if (feature.type === "pattern") openPattern(id);
      else if (feature.type === "hole") openHole(id);
      else if (feature.type === "extrude") openExtrude(id);
      else openBlend(feature.type, id);
    });
    item.append(name, detail, remove);
    ui.featureList.append(item);
  }
}

/** The setup whose toolpath is drawn, if any. */
let shownSetup: number | null = null;

function operationText(op: CamOperation): string {
  switch (op.type) {
    case "face":
      return "Face the stock";
    case "contour":
      return "Contour the outline";
    case "pocket":
      return "Pocket to a floor";
    case "holes":
      return "Holes: spot, drill, tap, ream";
    case "chamfer":
      return `Chamfer ${formatNumber(op.width)} mm`;
  }
}

/** Faces for CAM operations are picked on the part: what to say while picking. */
const pickPrompts: Record<string, string> = {
  contour: "Click the face whose outline the contour follows (usually the bottom). Esc cancels.",
  pocket: "Click the pocket's floor. Esc cancels.",
  chamfer: "Click the face whose edges get the chamfer (usually the top). Esc cancels.",
};

/** A CAM operation waiting for its face to be picked. */
let camPick: { setup: number; type: string } | null = null;

async function showToolpath(setup: number | null): Promise<void> {
  shownSetup = setup;
  if (setup === null) {
    viewer.setToolpath(null);
  } else {
    try {
      viewer.setToolpath(await toolpathPreview(setup));
    } catch (error) {
      viewer.setToolpath(null);
      shownSetup = null;
      throw error;
    }
  }
  if (current) renderSetups(current);
}

/**
 * What the Post… button writes for: a shipped machine (its post, with the
 * program checked against its limits first, decision 0015) or a bare post.
 */
interface Target {
  key: string;
  label: string;
  post: string;
  machine?: string;
}
const MAZAK_HCN_5000: Target = {
  key: "mazak-hcn-5000",
  label: "Mazak HCN-5000",
  post: "mazak-smoothg",
  machine: "mazak-hcn-5000",
};
const TARGETS: Target[] = [
  MAZAK_HCN_5000,
  { key: "fanuc-mill", label: "Fanuc", post: "fanuc-mill" },
  { key: "haas-mill", label: "Haas", post: "haas-mill" },
  { key: "linuxcnc-mill", label: "LinuxCNC", post: "linuxcnc-mill" },
  { key: "mazak-smoothg", label: "Mazak SmoothG", post: "mazak-smoothg" },
  { key: "grbl", label: "GRBL", post: "grbl" },
];
let chosenTarget = MAZAK_HCN_5000;

/** Posts one setup, or all of them as one program when `setup` is a list. */
async function postSetup(setup: number | number[], name: string): Promise<void> {
  const { post, machine } = chosenTarget;
  const which = Array.isArray(setup) ? { setups: setup } : { setup };
  const path = await save({
    title: "Save G-code",
    defaultPath: `${name}.nc`,
    filters: [{ name: "G-code", extensions: ["nc", "tap", "ngc"] }],
  });
  if (!path) return;
  const result = await request<{ gcode: string; notes: string[] }>({ method: "post", post, machine, ...which, path });
  const lines = result.gcode.split("\n").length;
  const notes = result.notes.length > 0 ? ` Notes: ${result.notes.join(" ")}` : "";
  showError(new Error(`Saved ${lines} lines of UNVERIFIED G-code. Prove it out carefully.${notes}`));
}

function renderSetups(state: DocumentState): void {
  const setups = state.document.cam?.setups ?? [];
  ui.setupList.replaceChildren();
  ui.noSetups.hidden = setups.length > 0;
  element("cam-post-all").hidden = setups.length < 2;
  for (const { id, setup } of setups) {
    const item = globalThis.document.createElement("li");
    item.classList.toggle("showing", shownSetup === id);
    const head = globalThis.document.createElement("div");
    head.className = "head";
    const name = globalThis.document.createElement("span");
    name.className = "name";
    const table = setup.index === undefined ? "" : ` · B${setup.index}`;
    name.textContent = `${setup.name} · G${53 + setup.work_offset}${table}`;
    const show = globalThis.document.createElement("button");
    show.type = "button";
    show.className = "secondary";
    show.textContent = shownSetup === id ? "Hide" : "Show";
    show.addEventListener("click", guarded(() => showToolpath(shownSetup === id ? null : id)));
    const post = globalThis.document.createElement("button");
    post.type = "button";
    post.className = "secondary";
    post.textContent = "Post…";
    post.addEventListener("click", guarded(() => postSetup(id, `${state.document.name} ${setup.name}`)));
    const controller = globalThis.document.createElement("select");
    controller.title = "Machine or controller the Post… button writes for";
    for (const target of TARGETS) {
      const option = globalThis.document.createElement("option");
      option.value = target.key;
      option.textContent = target.label;
      option.selected = target === chosenTarget;
      controller.append(option);
    }
    controller.addEventListener("change", () => {
      chosenTarget = TARGETS.find((t) => t.key === controller.value) ?? MAZAK_HCN_5000;
      if (current) renderSetups(current);
    });
    const remove = globalThis.document.createElement("button");
    remove.type = "button";
    remove.className = "secondary";
    remove.textContent = "×";
    remove.title = "Delete this setup";
    remove.addEventListener(
      "click",
      guarded(async () => {
        if (shownSetup === id) await showToolpath(null);
        render(await execute("cam.remove_setup", { setup: id }));
      }),
    );
    head.append(name, show, controller, post, remove);
    const ops = globalThis.document.createElement("ol");
    ops.className = "ops";
    for (const op of setup.operations) {
      const li = globalThis.document.createElement("li");
      li.textContent = operationText(op.operation);
      li.title = "Double-click to delete";
      li.addEventListener(
        "dblclick",
        guarded(async () => render(await execute("cam.remove_operation", { setup: id, operation: op.id }))),
      );
      ops.append(li);
    }
    const row = globalThis.document.createElement("div");
    row.className = "row";
    const kind = globalThis.document.createElement("select");
    for (const [value, label] of [
      ["face", "Face"],
      ["contour", "Contour…"],
      ["pocket", "Pocket…"],
      ["holes", "Holes"],
      ["chamfer", "Chamfer 0.5…"],
    ]) {
      const option = globalThis.document.createElement("option");
      option.value = value ?? "";
      option.textContent = label ?? "";
      kind.append(option);
    }
    const add = globalThis.document.createElement("button");
    add.type = "button";
    add.className = "secondary";
    add.textContent = "Add";
    add.addEventListener(
      "click",
      guarded(async () => {
        const type = kind.value;
        const prompt = pickPrompts[type];
        if (prompt) {
          camPick = { setup: id, type };
          viewer.canvas.style.cursor = "pointer";
          ui.viewportHint.textContent = prompt;
          ui.viewportHint.hidden = false;
          return;
        }
        render(await execute("cam.add_operation", { setup: id, type }));
        if (shownSetup === id) await showToolpath(id);
      }),
    );
    const detect = globalThis.document.createElement("button");
    detect.type = "button";
    detect.className = "secondary";
    detect.textContent = "Detect";
    detect.title = "Add operations for the holes, pockets and outside profile this setup can reach";
    detect.addEventListener(
      "click",
      guarded(async () => {
        const found = await request<{ operations: unknown[]; notes: string[] }>({ method: "detect", setup: id });
        if (found.operations.length > 0) {
          render(await execute("cam.add_operations", { setup: id, operations: found.operations }));
          if (shownSetup === id) await showToolpath(id);
        }
        const added =
          found.operations.length === 0
            ? "Nothing new to add: the setup already has everything detection finds."
            : `Added ${found.operations.length} operation${found.operations.length === 1 ? "" : "s"}.`;
        if (found.notes.length > 0 || found.operations.length === 0) {
          showError(new Error(`${added} ${found.notes.join(" ")}`.trim()));
        }
      }),
    );
    row.append(kind, add, detect);
    item.append(head, ops, row);
    ui.setupList.append(item);
  }
}

/** One line about a feature for the timeline. */
function featureDetail(feature: import("./api").Feature): string {
  if (feature.type === "extrude") {
    const fx = feature.distance_expression ? "ƒ " : "";
    return `${fx}${feature.operation === "cut" ? "cut " : ""}${formatNumber(feature.distance)} mm`;
  }
  if (feature.type === "chamfer") return `${formatNumber(feature.distance)} × 45°`;
  if (feature.type === "pattern") {
    const source = current?.document.features?.find((f) => f.id === feature.feature)?.feature.name ?? "?";
    return `${feature.count_x} × ${feature.count_y} of ${source}`;
  }
  if (feature.type === "fillet") return `R${formatNumber(feature.radius)}`;
  const { spec, points } = feature;
  const size = spec.thread ? spec.thread.size.replace(/x.*/, "") : `Ø${formatNumber(spec.diameter)}`;
  const depth = spec.depth === "through" ? "thru" : `${formatNumber(spec.depth.blind)} deep`;
  const cb = spec.counterbore ? " cbore" : "";
  return `${points.length} × ${size}${cb} ${depth}`;
}

/** Points no curve is drawn from: where holes go. */
function freePoints(id: string, state: DocumentState): number[] {
  const entities = state.document.sketches?.[id]?.sketch.entities ?? {};
  const used = new Set<number>();
  for (const e of Object.values(entities)) {
    const g = e.geometry;
    if (g.type === "line") [g.start, g.end].forEach((p) => used.add(p));
    if (g.type === "circle") used.add(g.center);
    if (g.type === "arc") [g.center, g.start, g.end].forEach((p) => used.add(p));
  }
  return Object.entries(entities)
    .filter(([key, e]) => e.geometry.type === "point" && !e.construction && !used.has(Number(key)))
    .map(([key]) => Number(key));
}

let editingHole: number | null = null;
let editingPattern: number | null = null;

function openPattern(feature: number | null): void {
  if (!current) return;
  closeExtrude();
  closeHole();
  closeBlend();
  editingPattern = feature;
  const features = current.document.features ?? [];
  ui.patternSource.replaceChildren(
    ...features
      .filter((f) => f.feature.type === "extrude" || f.feature.type === "hole")
      .map((f) => {
        const option = globalThis.document.createElement("option");
        option.value = String(f.id);
        option.textContent = f.feature.name;
        return option;
      }),
  );
  const existing = features.find((f) => f.id === feature)?.feature;
  if (existing?.type === "pattern") {
    ui.patternTitle.textContent = `Edit ${existing.name}`;
    ui.patternSource.value = String(existing.feature);
    ui.patternSource.disabled = true;
    ui.patternCountX.value = String(existing.count_x);
    ui.patternSpacingX.value = String(existing.spacing_x);
    ui.patternCountY.value = String(existing.count_y);
    ui.patternSpacingY.value = String(existing.spacing_y);
  } else {
    ui.patternTitle.textContent = "Pattern";
    ui.patternSource.disabled = false;
    const last = ui.patternSource.options[ui.patternSource.options.length - 1];
    if (last) ui.patternSource.value = last.value;
  }
  ui.patternDialog.hidden = false;
  ui.patternSource.focus();
}

function closePattern(): void {
  ui.patternDialog.hidden = true;
  editingPattern = null;
}

/** The chamfer or fillet being set up: which kind, the picked faces, and
 * the feature being edited (then no picking). */
let blending: { kind: "chamfer" | "fillet"; faces: number[]; edges: number[]; editing: number | null } | null =
  null;

function syncBlend(): void {
  if (!blending) return;
  const faces = blending.faces.length;
  const edges = blending.edges.length;
  const picked = [
    faces ? `${faces} face${faces === 1 ? "" : "s"}` : "",
    edges ? `${edges} edge${edges === 1 ? "" : "s"}` : "",
  ]
    .filter(Boolean)
    .join(" and ");
  ui.blendPicked.textContent =
    blending.editing !== null
      ? "The edges stay as picked."
      : picked === ""
        ? "Click edges, or faces to take all their edges. Click again to unpick."
        : `${picked} picked. Click more, or OK.`;
  viewer.highlightFaces(blending.faces);
  viewer.highlightEdges(blending.edges);
}

function openBlend(kind: "chamfer" | "fillet", feature: number | null): void {
  if (!current) return;
  closeExtrude();
  closeHole();
  const existing = current.document.features?.find((f) => f.id === feature)?.feature;
  blending = { kind, faces: [], edges: [], editing: feature };
  const label = kind === "chamfer" ? "Chamfer" : "Fillet";
  ui.blendTitle.textContent = existing ? `Edit ${existing.name}` : label;
  ui.blendSizeLabel.textContent = kind === "chamfer" ? "Distance (mm)" : "Radius (mm)";
  if (existing?.type === "chamfer") ui.blendSize.value = String(existing.distance);
  else if (existing?.type === "fillet") ui.blendSize.value = String(existing.radius);
  else ui.blendSize.value = kind === "chamfer" ? "0.5" : "2";
  viewer.canvas.style.cursor = feature === null ? "pointer" : "";
  ui.blendDialog.hidden = false;
  syncBlend();
}

function closeBlend(): void {
  blending = null;
  ui.blendDialog.hidden = true;
  viewer.canvas.style.cursor = "";
  viewer.highlightFaces([]);
  viewer.highlightEdges([]);
}

function syncHoleFields(): void {
  const type = ui.holeType.value;
  for (const field of ui.holeDialog.querySelectorAll<HTMLElement>("[data-for]")) {
    field.hidden = !(field.dataset["for"] ?? "").split(" ").includes(type);
  }
  ui.holeDepth.disabled = ui.holeThrough.checked;
  const id = ui.holeSketch.value;
  const count = current ? freePoints(id, current).length : 0;
  ui.holePoints.textContent =
    editingHole !== null
      ? "The hole keeps its points."
      : count === 0
        ? "This sketch has no free-standing points. Add points where the holes go."
        : `${count} hole${count === 1 ? "" : "s"}, one at each free-standing point.`;
}

function openHole(feature: number | null): void {
  if (!current) return;
  closeBlend();
  closePattern();
  if (editor.editing !== null) {
    editor.close();
    updateSketchUi();
  }
  closeExtrude();
  editingHole = feature;
  const sketches = Object.entries(current.document.sketches ?? {});
  ui.holeSketch.replaceChildren(
    ...sketches.map(([id, record]) => {
      const option = globalThis.document.createElement("option");
      option.value = id;
      option.textContent = `${record.name} (${planeLabel(record.plane)}, ${freePoints(id, current!).length} points)`;
      return option;
    }),
  );
  const existing = current.document.features?.find((f) => f.id === feature)?.feature;
  if (existing?.type === "hole") {
    const { spec } = existing;
    ui.holeTitle.textContent = `Edit ${existing.name}`;
    ui.holeSketch.value = String(existing.sketch);
    ui.holeSketch.disabled = true;
    ui.holeType.value = spec.thread ? "tapped" : spec.counterbore ? "counterbore" : "simple";
    ui.holeDiameter.value = String(spec.diameter);
    if (spec.thread) {
      ui.holeThread.value = spec.thread.size;
      ui.holeThreadDepth.value = String(spec.thread.depth);
    }
    if (spec.counterbore) {
      ui.holeCbDiameter.value = String(spec.counterbore.diameter);
      ui.holeCbDepth.value = String(spec.counterbore.depth);
    }
    ui.holeThrough.checked = spec.depth === "through";
    if (spec.depth !== "through") ui.holeDepth.value = String(spec.depth.blind);
    ui.holeTip.value = String(spec.tip_angle);
    ui.holeFit.value = spec.fit ?? "";
  } else {
    ui.holeTitle.textContent = "Hole";
    ui.holeSketch.disabled = false;
    // The newest sketch with points is the likely one.
    const withPoints = sketches.filter(([id]) => freePoints(id, current!).length > 0).at(-1) ?? sketches.at(-1);
    if (withPoints) ui.holeSketch.value = withPoints[0];
  }
  syncHoleFields();
  ui.holeDialog.hidden = false;
  ui.holeType.focus();
}

function closeHole(): void {
  ui.holeDialog.hidden = true;
  editingHole = null;
}

function holeArgs(): Record<string, unknown> {
  const type = ui.holeType.value;
  const args: Record<string, unknown> = {
    depth: ui.holeThrough.checked ? "through" : Number(ui.holeDepth.value),
    tip_angle: Number(ui.holeTip.value),
  };
  if (type === "tapped") {
    args["thread"] = ui.holeThread.value;
    args["thread_depth"] = Number(ui.holeThreadDepth.value);
  } else {
    args["diameter"] = Number(ui.holeDiameter.value);
  }
  if (type === "counterbore") {
    args["counterbore_diameter"] = Number(ui.holeCbDiameter.value);
    args["counterbore_depth"] = Number(ui.holeCbDepth.value);
  }
  const fit = ui.holeFit.value.trim();
  if (fit) args["fit"] = fit;
  return args;
}

/** The feature being edited by the dialog, or null for a new extrude. */
let editingFeature: number | null = null;

function openExtrude(feature: number | null): void {
  if (!current) return;
  closeHole();
  closeBlend();
  closePattern();
  if (editor.editing !== null) {
    editor.close();
    updateSketchUi();
  }
  editingFeature = feature;
  const sketches = Object.entries(current.document.sketches ?? {});
  ui.extrudeSketch.replaceChildren(
    ...sketches.map(([id, record]) => {
      const option = globalThis.document.createElement("option");
      option.value = id;
      option.textContent = `${record.name} (${planeLabel(record.plane)})`;
      return option;
    }),
  );
  const existing = current.document.features?.find((f) => f.id === feature)?.feature;
  if (existing?.type === "extrude") {
    ui.extrudeTitle.textContent = `Edit ${existing.name}`;
    ui.extrudeSketch.value = String(existing.sketch);
    ui.extrudeSketch.disabled = true;
    ui.extrudeDistance.value = existing.distance_expression ?? String(existing.distance);
    ui.extrudeOperation.value = existing.operation;
  } else {
    ui.extrudeTitle.textContent = "Extrude";
    ui.extrudeSketch.disabled = false;
    // The most recent sketch is the likely one.
    const last = sketches.at(-1);
    if (last) ui.extrudeSketch.value = last[0];
    ui.extrudeOperation.value = (current.document.features?.length ?? 0) === 0 ? "join" : ui.extrudeOperation.value;
  }
  ui.extrudeDialog.hidden = false;
  ui.extrudeDistance.focus();
  ui.extrudeDistance.select();
}

function closeExtrude(): void {
  ui.extrudeDialog.hidden = true;
  editingFeature = null;
}

function updateSketchUi(): void {
  const editing = editor.editing;
  if (editing !== null) {
    closeExtrude();
    closeHole();
    closeBlend();
    closePattern();
  }
  ui.sketchTools.hidden = editing === null;
  ui.sketchTools.parentElement?.classList.toggle("sketching", editing !== null);
  ui.viewportHint.hidden = editing !== null || ui.modelEmpty.hidden;
  for (const button of ui.sketchTools.querySelectorAll<HTMLButtonElement>("[data-tool]")) {
    button.classList.toggle("active", button.dataset["tool"] === editor.tool);
  }
  for (const item of ui.sketchList.querySelectorAll("li")) item.classList.remove("editing");
  if (editing === null || !current) {
    ui.statusSketch.hidden = true;
    return;
  }
  const id = String(editing);
  const index = Object.keys(current.document.sketches ?? {}).indexOf(id);
  ui.sketchList.querySelectorAll("li")[index]?.classList.add("editing");
  const name = current.document.sketches?.[id]?.name ?? "Sketch";
  ui.statusSketch.textContent = `${name}: ${freedomText(id, current)}`;
  ui.statusSketch.hidden = false;
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

element("fit").addEventListener("click", () => {
  if (editor.editing !== null) editor.fit();
  else viewer.fit();
});

ui.extrudeButton.addEventListener("click", () => openExtrude(null));
ui.holeButton.addEventListener("click", () => openHole(null));
element("hole-cancel").addEventListener("click", closeHole);
for (const input of [ui.holeType, ui.holeThrough, ui.holeSketch]) input.addEventListener("change", syncHoleFields);
ui.holeDialog.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeHole();
});
ui.holeDialog.addEventListener("submit", (event) => {
  event.preventDefault();
  guarded(async () => {
    const next =
      editingHole === null
        ? await execute("feature.hole", { sketch: Number(ui.holeSketch.value), ...holeArgs() })
        : await execute("feature.edit_hole", { feature: editingHole, ...holeArgs() });
    closeHole();
    render(next);
  })();
});
element("extrude-cancel").addEventListener("click", closeExtrude);

/** On a face, the normal points out of the part: a cut goes in (negative)
 * and a join goes out (positive). Switching the operation flips the sign
 * to match; the user can still type either. */
ui.extrudeOperation.addEventListener("change", () => {
  const plane = current?.document.sketches?.[ui.extrudeSketch.value]?.plane;
  if (typeof plane === "string") return;
  const value = Number(ui.extrudeDistance.value);
  if (!Number.isFinite(value)) return; // an expression: leave it as typed
  const distance = Math.abs(value);
  ui.extrudeDistance.value = String(ui.extrudeOperation.value === "cut" ? -distance : distance);
});
ui.extrudeDialog.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeExtrude();
});
ui.extrudeDialog.addEventListener("submit", (event) => {
  event.preventDefault();
  guarded(async () => {
    // A number, or an expression of the parameters.
    const text = ui.extrudeDistance.value.trim();
    const distance = Number.isFinite(Number(text)) && text !== "" ? Number(text) : text;
    const operation = ui.extrudeOperation.value;
    const next =
      editingFeature === null
        ? await execute("feature.extrude", {
            sketch: Number(ui.extrudeSketch.value),
            distance,
            operation,
          })
        : await execute("feature.edit_extrude", { feature: editingFeature, distance, operation });
    closeExtrude();
    render(next);
  })();
});

ui.newSketch.addEventListener("click", () => {
  const open = ui.planeMenu.hidden;
  ui.planeMenu.hidden = !open;
  ui.newSketch.setAttribute("aria-expanded", String(open));
});
for (const button of ui.planeMenu.querySelectorAll<HTMLButtonElement>("[data-plane]")) {
  button.addEventListener(
    "click",
    guarded(async () => {
      ui.planeMenu.hidden = true;
      ui.newSketch.setAttribute("aria-expanded", "false");
      const next = await execute("sketch.create", { plane: button.dataset["plane"] });
      render(next);
      const ids = Object.keys(next.document.sketches ?? {}).map(Number);
      const created = Math.max(...ids);
      editor.open(created, next);
      editor.setTool("line");
    }),
  );
}
/** Face picking for "New sketch → On a face". */
let pickingFace = false;

function stopPicking(): void {
  pickingFace = false;
  viewer.highlightFace(null);
  viewer.canvas.style.cursor = "";
  ui.viewportHint.hidden = true;
}

/** A name that belongs to this face alone, so the sketch finds exactly it. */
function uniqueFaceName(face: number): string | null {
  const faces = current?.body?.faces ?? [];
  const names = faces[face] ?? [];
  return names.find((name) => faces.filter((other) => other.includes(name)).length === 1) ?? null;
}

element("sketch-on-face").addEventListener("click", () => {
  ui.planeMenu.hidden = true;
  if (!current?.body) {
    showError(new Error("There is no part yet. Extrude a sketch first, then sketch on one of its faces."));
    return;
  }
  pickingFace = true;
  viewer.canvas.style.cursor = "pointer";
  ui.viewportHint.textContent = "Click a flat face to sketch on. Esc cancels.";
  ui.viewportHint.hidden = false;
});

viewer.canvas.addEventListener("pointermove", (event) => {
  if (pickingFace) viewer.highlightFace(viewer.faceAt(event.clientX, event.clientY));
  else if (blending && blending.editing === null) {
    const edge = viewer.edgeAt(event.clientX, event.clientY);
    viewer.highlightEdges(blending.edges, edge);
    viewer.highlightFaces(blending.faces, edge === null ? viewer.faceAt(event.clientX, event.clientY) : null);
  }
});

viewer.canvas.addEventListener("click", (event) => {
  if (!blending || blending.editing !== null) return;
  // An edge near the pointer wins over the face under it.
  const edge = viewer.edgeAt(event.clientX, event.clientY);
  if (edge !== null) {
    blending.edges = blending.edges.includes(edge)
      ? blending.edges.filter((e) => e !== edge)
      : [...blending.edges, edge];
  } else {
    const face = viewer.faceAt(event.clientX, event.clientY);
    if (face === null) return;
    blending.faces = blending.faces.includes(face)
      ? blending.faces.filter((f) => f !== face)
      : [...blending.faces, face];
  }
  syncBlend();
});

ui.chamferButton.addEventListener("click", () => openBlend("chamfer", null));
element("cam-add-top").addEventListener(
  "click",
  guarded(async () => {
    // The first setup also brings the test block's stock allowances.
    if (!current?.document.cam?.stock) render(await execute("cam.set_stock", { side: 2, top: 2, bottom: 3 }));
    const setups = current?.document.cam?.setups?.length ?? 0;
    const table = element<HTMLSelectElement>("cam-turn").value === "table";
    // On the B table each side keeps its own offset, set at its angle.
    render(
      await execute(
        "cam.add_setup",
        table ? { side: "top", index: 0, work_offset: Math.min(setups + 1, 6) } : { side: "top" },
      ),
    );
  }),
);
element("cam-add-bottom").addEventListener(
  "click",
  guarded(async () => {
    if (!current?.document.cam?.stock) render(await execute("cam.set_stock", { side: 2, top: 2, bottom: 3 }));
    const setups = current?.document.cam?.setups?.length ?? 0;
    const table = element<HTMLSelectElement>("cam-turn").value === "table";
    // The B table turns a part standing on the pallet about its upright
    // axis, model Y.
    const turn = table ? { index: 180, flip_about: "y" } : {};
    render(await execute("cam.add_setup", { side: "bottom", work_offset: Math.min(setups + 1, 6), ...turn }));
  }),
);
element("cam-post-all").addEventListener(
  "click",
  guarded(async () => {
    const setups = current?.document.cam?.setups ?? [];
    await postSetup(
      setups.map((s) => s.id),
      `${current?.document.name ?? "part"} all sides`,
    );
  }),
);
viewer.canvas.addEventListener("pointermove", (event) => {
  if (camPick) viewer.highlightFace(viewer.faceAt(event.clientX, event.clientY));
});
viewer.canvas.addEventListener("click", (event) => {
  if (!camPick) return;
  const face = viewer.faceAt(event.clientX, event.clientY);
  if (face === null) return;
  const pick = camPick;
  camPick = null;
  viewer.canvas.style.cursor = "";
  viewer.highlightFace(null);
  ui.viewportHint.hidden = true;
  guarded(async () => {
    const name = uniqueFaceName(face);
    if (!name) throw new Error("That face has no name of its own yet.");
    const args: Record<string, unknown> = { setup: pick.setup, type: pick.type };
    if (pick.type === "pocket") args["floor"] = name;
    else args["face"] = name;
    if (pick.type === "chamfer") args["width"] = 0.5;
    render(await execute("cam.add_operation", args));
    if (shownSetup === pick.setup) await showToolpath(pick.setup);
  })();
});
window.addEventListener("keydown", (event) => {
  if (camPick && event.key === "Escape") {
    camPick = null;
    viewer.canvas.style.cursor = "";
    viewer.highlightFace(null);
    ui.viewportHint.hidden = true;
  }
});
ui.patternButton.addEventListener("click", () => openPattern(null));
element("pattern-cancel").addEventListener("click", closePattern);
ui.patternDialog.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closePattern();
});
ui.patternDialog.addEventListener("submit", (event) => {
  event.preventDefault();
  guarded(async () => {
    const grid = {
      count_x: Number(ui.patternCountX.value),
      spacing_x: Number(ui.patternSpacingX.value),
      count_y: Number(ui.patternCountY.value),
      spacing_y: Number(ui.patternSpacingY.value),
    };
    const next =
      editingPattern === null
        ? await execute("feature.pattern", { feature: Number(ui.patternSource.value), ...grid })
        : await execute("feature.edit_pattern", { feature: editingPattern, ...grid });
    closePattern();
    render(next);
  })();
});
ui.filletButton.addEventListener("click", () => openBlend("fillet", null));
element("blend-cancel").addEventListener("click", closeBlend);
ui.blendDialog.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeBlend();
});
ui.blendDialog.addEventListener("submit", (event) => {
  event.preventDefault();
  guarded(async () => {
    if (!blending) return;
    const size = Number(ui.blendSize.value);
    let next: DocumentState;
    if (blending.editing !== null) {
      next = await execute("feature.edit_blend", { feature: blending.editing, size });
    } else {
      if (blending.faces.length === 0 && blending.edges.length === 0) {
        throw new Error("Pick at least one edge or face on the part first.");
      }
      const faces = blending.faces.map((f) => {
        const name = uniqueFaceName(f);
        if (!name) throw new Error("One of the picked faces has no name of its own yet.");
        return name;
      });
      // An edge is named by its two faces; that pair must meet only there.
      const joins = current?.body?.edges ?? [];
      const edges = blending.edges.map((e) => {
        const [a, b] = joins[e] ?? [-1, -1];
        const na = uniqueFaceName(a);
        const nb = uniqueFaceName(b);
        const same = joins.filter(([x, y]) => (x === a && y === b) || (x === b && y === a)).length;
        if (!na || !nb || a === b || same !== 1) {
          throw new Error("One of the picked edges can't be named on its own yet (its faces meet at more than one edge). Pick its face instead.");
        }
        return [na, nb];
      });
      const key = blending.kind === "chamfer" ? "distance" : "radius";
      const args: Record<string, unknown> = { [key]: size };
      if (faces.length > 0) args["faces"] = faces;
      if (edges.length > 0) args["edges"] = edges;
      next = await execute(`feature.${blending.kind}`, args);
    }
    closeBlend();
    render(next);
  })();
});

viewer.canvas.addEventListener("click", (event) => {
  if (!pickingFace) return;
  const face = viewer.faceAt(event.clientX, event.clientY);
  if (face === null) return;
  stopPicking();
  guarded(async () => {
    const name = uniqueFaceName(face);
    if (!name) throw new Error("That face has no name of its own yet, so a sketch could not find it again.");
    const next = await execute("sketch.create", { face: name });
    const ids = Object.keys(next.document.sketches ?? {}).map(Number);
    const created = Math.max(...ids);
    const frame = next.sketch_frames[String(created)];
    if (frame?.state === "failed") {
      // Not a flat face: take the sketch back out and say why.
      render(await request<DocumentState>({ method: "undo" }));
      throw new Error(`Can't sketch there: ${frame.reason}.`);
    }
    render(next);
    editSketch(created, next);
    editor.setTool("line");
  })();
});

window.addEventListener("keydown", (event) => {
  if (pickingFace && event.key === "Escape") stopPicking();
});

globalThis.document.addEventListener("pointerdown", (event) => {
  const target = event.target as Node;
  if (!ui.planeMenu.hidden && !ui.planeMenu.contains(target) && !ui.newSketch.contains(target)) {
    ui.planeMenu.hidden = true;
    ui.newSketch.setAttribute("aria-expanded", "false");
  }
});

for (const button of ui.sketchTools.querySelectorAll<HTMLButtonElement>("[data-tool]")) {
  button.addEventListener("click", () => editor.setTool(button.dataset["tool"] as Tool));
}
for (const button of ui.sketchTools.querySelectorAll<HTMLButtonElement>("[data-constraint]")) {
  button.addEventListener(
    "click",
    guarded(() => editor.constrain(button.dataset["constraint"] ?? "")),
  );
}
element("construction").addEventListener("click", guarded(() => editor.toggleConstruction()));
element("sketch-fit").addEventListener("click", () => editor.fit());
element("finish-sketch").addEventListener("click", () => {
  editor.close();
  updateSketchUi();
  viewer.fit();
});

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
const toolKeys: Record<string, Tool> = {
  l: "line",
  r: "rectangle",
  c: "circle",
  a: "arc",
  p: "point",
  d: "dimension",
};

window.addEventListener("keydown", (event) => {
  const target = event.target as HTMLElement | null;
  if (target && (target.tagName === "INPUT" || target.tagName === "SELECT")) return;
  const mod = event.metaKey || event.ctrlKey;
  if (!mod && editor.editing !== null) {
    const key = event.key.toLowerCase();
    const tool = toolKeys[key];
    if (event.key === "Escape") editor.escape();
    else if (event.key === "Delete" || event.key === "Backspace") guarded(() => editor.deleteSelection())();
    else if (tool) editor.setTool(tool);
    else if (key === "x") guarded(() => editor.toggleConstruction())();
    else if (key === "f") editor.fit();
    else return;
    event.preventDefault();
    return;
  }
  if (!mod && editor.editing === null && event.key.toLowerCase() === "e" && !ui.extrudeButton.disabled) {
    event.preventDefault();
    openExtrude(null);
    return;
  }
  if (!mod && editor.editing === null && event.key.toLowerCase() === "h" && !ui.holeButton.disabled) {
    event.preventDefault();
    openHole(null);
    return;
  }
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
