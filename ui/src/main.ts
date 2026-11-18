// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The app shell: wires the page to the API. Every change to the document is
// an API request, so undo, redo and unsaved-change tracking come from the
// core, not from here.

import "./boot"; // first: fills the page's [[id]] markers before anything else runs
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import {
  ApiFailure,
  bodyEdges,
  bodyMesh,
  referenceMesh,
  type DocumentState,
  planeLabel,
  toolpathPreview,
  type CamOperation,
  execute,
  type Parameter,
  request,
  state,
  type ProjectThumbnail,
  type StepInfo,
  stepPreview,
  simulationMesh,
} from "./api";
import { firstSentence, matchEntries, type PaletteEntry } from "./palette";
import { operationRange, type Toolpath, visibleMoves } from "./scrub";
import { SketchEditor, type Tool } from "./sketch";
import { ui as text } from "./strings";
import { type Layer, type ViewName, Viewer } from "./viewer";

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
  revolveButton: element<HTMLButtonElement>("revolve"),
  revolveDialog: element<HTMLFormElement>("revolve-dialog"),
  revolveTitle: element<HTMLHeadingElement>("revolve-title"),
  revolveSketch: element<HTMLSelectElement>("revolve-sketch"),
  revolveAxis: element<HTMLSelectElement>("revolve-axis"),
  revolveAngle: element<HTMLInputElement>("revolve-angle"),
  revolveOperation: element<HTMLSelectElement>("revolve-operation"),
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
  circularButton: element<HTMLButtonElement>("circular-pattern"),
  circularDialog: element<HTMLFormElement>("circular-dialog"),
  circularTitle: element<HTMLHeadingElement>("circular-title"),
  circularSource: element<HTMLSelectElement>("circular-source"),
  circularCount: element<HTMLInputElement>("circular-count"),
  circularAngle: element<HTMLInputElement>("circular-angle"),
  circularCenter: element<HTMLSelectElement>("circular-center"),
  filletButton: element<HTMLButtonElement>("fillet"),
  blendDialog: element<HTMLFormElement>("blend-dialog"),
  blendTitle: element<HTMLHeadingElement>("blend-title"),
  blendSizeLabel: element<HTMLSpanElement>("blend-size-label"),
  blendSize: element<HTMLInputElement>("blend-size"),
  blendPicked: element<HTMLParagraphElement>("blend-picked"),
  toast: element<HTMLDivElement>("toast"),
};

const viewer = new Viewer(element("viewport"));

// Theme: follows the system until a choice is made in the app bar, which
// this computer remembers.
type ThemeChoice = "system" | "light" | "dark";
const THEME_KEY = "cndt.theme";
const systemLight = globalThis.matchMedia?.("(prefers-color-scheme: light)");
function savedTheme(): ThemeChoice {
  try {
    const value = globalThis.localStorage?.getItem(THEME_KEY);
    return value === "light" || value === "dark" ? value : "system";
  } catch {
    return "system";
  }
}
let themeChoice: ThemeChoice = savedTheme();
function applyTheme(): void {
  const root = globalThis.document.documentElement;
  if (themeChoice === "system") delete root.dataset.theme;
  else root.dataset.theme = themeChoice;
  const light = themeChoice === "light" || (themeChoice === "system" && systemLight?.matches === true);
  viewer.setTheme(light ? "light" : "dark");
  const next: Record<ThemeChoice, ThemeChoice> = { system: "light", light: "dark", dark: "system" };
  const button = element("theme");
  button.title = text.themeTitle(text.themeNames[themeChoice], text.themeNames[next[themeChoice]]);
  button.setAttribute("aria-label", button.title);
}
element("theme").addEventListener("click", () => {
  const next: Record<ThemeChoice, ThemeChoice> = { system: "light", light: "dark", dark: "system" };
  themeChoice = next[themeChoice];
  try {
    if (themeChoice === "system") globalThis.localStorage?.removeItem(THEME_KEY);
    else globalThis.localStorage?.setItem(THEME_KEY, themeChoice);
  } catch {
    // Not remembered; the choice still applies now.
  }
  applyTheme();
});
systemLight?.addEventListener("change", applyTheme);
applyTheme();
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
        ? text.conflictNamed
        : text.conflict;
    }
    if (error.kind === "redundant") {
      return named
        ? text.redundantNamed
        : text.redundant;
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
  element("tree-doc").textContent = doc.name;
  ui.savedState.textContent = dirty ? text.unsaved : text.saved;
  ui.savedState.classList.toggle("dirty", dirty);
  ui.undo.disabled = undo === null;
  ui.undo.title = undo ? text.undoTitle(undo) : text.nothingToUndo;
  ui.redo.disabled = redo === null;
  ui.redo.title = redo ? text.redoTitle(redo) : text.nothingToRedo;
  ui.statusPath.textContent = path ?? text.notSavedYet;
  ui.statusUndo.textContent = undo ? text.lastChange(undo) : "";

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
    remove.setAttribute("aria-label", text.remove(name));
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
  renderTools(next);
  void showBody(next);
  editor.update(next);
  updateSketchUi();

  const title = text.windowTitle(doc.name, dirty);
  appWindow.setTitle(title).catch(() => {
    // The title is cosmetic; never interrupt work over it.
  });
}

const statusText: Record<string, string> = {
  fully: text.fullyConstrained,
  over: text.overConstrained,
  unknown: text.tooLarge,
};

function freedomText(id: string, state: DocumentState): string {
  const freedom = state.freedom[id];
  if (!freedom) return "";
  const entities = state.document.sketches?.[id]?.sketch.entities ?? {};
  if (Object.keys(entities).length === 0) return text.empty;
  if (freedom.status === "under") {
    const n = freedom.dof ?? 0;
    return text.degreesOfFreedom(n);
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
    item.title = text.clickToEdit;
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
    remove.setAttribute("aria-label", text.delete(record.name));
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

/** The reference meshes being fetched, so a caller can fit the view after. */
let referencesShown: Promise<void> = Promise.resolve();

/** Fetches and shows the reference meshes, and lists them with their sizes. */
async function showReferences(state: DocumentState): Promise<void> {
  const references = state.references ?? {};
  const ids = Object.keys(references).map(Number);
  const list = element("reference-list");
  list.replaceChildren();
  element("references-node").hidden = ids.length === 0;
  for (const id of ids) {
    const reference = references[String(id)]!;
    const feature = state.document.features?.find((f) => f.id === id)?.feature;
    const item = globalThis.document.createElement("li");
    const [x, y, z] = reference.size.map((v) => formatNumber(v));
    item.textContent = text.meshSize(feature?.name ?? text.mesh, x, y, z);
    item.title = text.meshDetail(feature && "source" in feature ? feature.source : "", reference.triangles, reference.bounds.min, reference.bounds.max);
    list.append(item);
  }
  try {
    viewer.setReferences(await Promise.all(ids.map((id) => referenceMesh(id))));
  } catch (error) {
    showError(error);
  }
}

/** Fetches and shows the rebuilt part when it changed. */
async function showBody(state: DocumentState): Promise<void> {
  if (state.body_revision === shownRevision) return;
  shownRevision = state.body_revision;
  referencesShown = showReferences(state);
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
    simulatedSetup = null;
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
    [text.size, text.mmSize(size)],
    [text.volume, text.cm3(formatNumber(Number((body.volume / 1000).toFixed(3))))],
    [text.faces, String(body.faces.length)],
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
  ui.revolveButton.disabled = ui.extrudeButton.disabled;
  ui.holeButton.disabled = !state.body;
  ui.chamferButton.disabled = !state.body;
  ui.patternButton.disabled = !(state.document.features ?? []).some(
    (f) => f.feature.type === "extrude" || f.feature.type === "hole",
  );
  ui.circularButton.disabled = !(state.document.features ?? []).some(
    (f) => f.feature.type === "extrude" || f.feature.type === "hole" || f.feature.type === "revolve",
  );
  ui.filletButton.disabled = !state.body;
  const built = state.rollback ?? features.length;
  for (const [index, { id, feature }] of features.entries()) {
    if (index === built) ui.featureList.append(rollbackMarker());
    const status = state.feature_status[String(id)];
    const item = globalThis.document.createElement("li");
    const failed = status?.state === "failed";
    item.classList.toggle("failed", failed);
    item.title = failed
      ? text.needsRepair(feature.name, status.reason)
      : text.featureTitle(feature.name, featureDetail(feature, id), feature.type === "import" || feature.type === "reference");
    item.tabIndex = 0;
    // The same icon as the tool that made it.
    const icon = globalThis.document.createElement("span");
    icon.className = "icon";
    const tool =
      feature.type === "extrude"
        ? ui.extrudeButton
        : feature.type === "revolve"
          ? ui.revolveButton
          : globalThis.document.getElementById(
            feature.type === "import" ? "insert-step" : feature.type === "reference" ? "import-mesh" : feature.type,
          );
    const svg = tool?.querySelector("svg");
    if (svg) icon.append(svg.cloneNode(true));
    const name = globalThis.document.createElement("span");
    name.className = "name";
    name.textContent = failed ? `⚠ ${feature.name}` : feature.name;
    item.setAttribute("aria-label", item.title);
    const remove = globalThis.document.createElement("button");
    remove.type = "button";
    remove.setAttribute("aria-label", text.delete(feature.name));
    remove.innerHTML =
      '<svg aria-hidden="true" viewBox="0 0 20 20"><path d="M5 5l10 10M15 5L5 15" /></svg>';
    remove.addEventListener("click", (event) => {
      event.stopPropagation();
      guarded(async () => render(await execute("feature.delete", { feature: id })))();
    });
    item.addEventListener("click", () => {
      if (feature.type === "import" || feature.type === "reference") return; // nothing to edit: re-import to change it
      if (feature.type === "pattern") openPattern(id);
      else if (feature.type === "circular_pattern") openCircular(id);
      else if (feature.type === "hole") openHole(id);
      else if (feature.type === "extrude") openExtrude(id);
      else if (feature.type === "revolve") openRevolve(id);
      else openBlend(feature.type, id);
    });
    item.addEventListener("keydown", (event) => {
      if (event.key === "Enter") item.click();
    });
    item.append(icon, name, remove);
    // Past the marker: not built, as if not there yet.
    item.classList.toggle("rolled", index >= built);
    ui.featureList.append(item);
  }
  if (features.length > 0 && built === features.length) ui.featureList.append(rollbackMarker());
  for (const id of ["tl-start", "tl-back"]) element<HTMLButtonElement>(id).disabled = built === 0;
  for (const id of ["tl-forward", "tl-end"]) element<HTMLButtonElement>(id).disabled = built === features.length;
}

/** Moves the history marker: the part as it was after `count` features. */
async function rollTo(count: number): Promise<void> {
  const total = current?.document.features?.length ?? 0;
  const to = Math.max(0, Math.min(count, total));
  render(await request<DocumentState>(to >= total ? { method: "rollback" } : { method: "rollback", to }));
}

/** The draggable marker in the timeline: features after it are not built. */
function rollbackMarker(): HTMLLIElement {
  const marker = globalThis.document.createElement("li");
  marker.className = "rollback-marker";
  marker.title = text.historyMarkerTitle;
  marker.setAttribute("aria-label", text.historyMarker);
  marker.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    marker.setPointerCapture(event.pointerId);
    marker.classList.add("dragging");
    const chips = () => [...ui.featureList.querySelectorAll<HTMLLIElement>("li:not(.rollback-marker)")];
    // The gap nearest the pointer: how many chips lie left of it.
    const gapAt = (x: number) => chips().filter((chip) => {
      const r = chip.getBoundingClientRect();
      return r.left + r.width / 2 < x;
    }).length;
    const move = (e: PointerEvent) => {
      marker.style.transform = `translateX(${e.clientX - event.clientX}px)`;
    };
    const up = (e: PointerEvent) => {
      marker.removeEventListener("pointermove", move);
      marker.removeEventListener("pointerup", up);
      marker.classList.remove("dragging");
      marker.style.transform = "";
      guarded(() => rollTo(gapAt(e.clientX)))();
    };
    marker.addEventListener("pointermove", move);
    marker.addEventListener("pointerup", up);
  });
  return marker;
}

/** The setup whose toolpath is drawn, if any. */
let shownSetup: number | null = null;
/** The setup whose simulated stock is drawn, if any. */
let simulatedSetup: number | null = null;

interface Finding {
  operation: string;
  step: number;
  depth: number;
}
interface SimReport {
  collisions: Finding[];
  gouges: Finding[];
  left: number;
  cut_minutes: number;
}

/** What a stock simulation found, in a sentence or two. */
function simulationText(report: SimReport): string {
  const mm = (v: number) => text.mm(formatNumber(Math.round(v * 100) / 100));
  const list = (items: Finding[]) => items.map((f) => text.findingDepth(f.operation, mm(f.depth))).join(", ");
  const parts = [
    report.collisions.length === 0 ? text.noRapidTouches : text.rapidsIntoStock(list(report.collisions)),
    report.gouges.length === 0 ? text.nothingCutsBelow : text.cutsBelowModel(list(report.gouges)),
    report.left > 0.05 ? text.materialLeft(mm(report.left)) : text.machinedToSize,
    text.cuttingTime(formatNumber(Math.round(report.cut_minutes * 10) / 10)),
  ];
  return text.simulated(parts.join(" "));
}

async function simulateSetup(setup: number | null): Promise<void> {
  simulatedSetup = null;
  viewer.setStock(null);
  if (setup !== null) {
    const report = await request<SimReport>({ method: "simulate", setup, cell: 0.5 });
    viewer.setStock(await simulationMesh(setup));
    simulatedSetup = setup;
    showError(new Error(simulationText(report)));
  }
  if (current) renderSetups(current);
}

function operationText(op: CamOperation): string {
  switch (op.type) {
    case "face":
      return text.opFace;
    case "contour":
      return text.opContour;
    case "pocket":
      return text.opPocket;
    case "holes":
      return text.opHoles;
    case "chamfer":
      return text.opChamfer(formatNumber(op.width));
  }
}

/** Faces for CAM operations are picked on the part: what to say while picking. */
const pickPrompts: Record<string, string> = {
  contour: text.pickContour,
  pocket: text.pickPocket,
  chamfer: text.pickChamfer,
};

/** A CAM operation waiting for its face to be picked. */
let camPick: { setup: number; type: string } | null = null;

/** The drawn toolpath and what of it is shown: one operation or all, and how many moves. */
let shownPath: Toolpath | null = null;
let scrubOperation: number | null = null;
let scrubMoves: number | null = null;

function drawScrubbed(): void {
  viewer.setToolpath(shownPath ? visibleMoves(shownPath, scrubOperation, scrubMoves) : null);
}

async function showToolpath(setup: number | null): Promise<void> {
  const same = setup !== null && setup === shownSetup;
  shownSetup = setup;
  if (setup === null) {
    shownPath = null;
    viewer.setToolpath(null);
  } else {
    try {
      shownPath = await toolpathPreview(setup);
      // Keep the scrub position across a refresh of the same setup, if it still fits.
      if (!same || (scrubOperation !== null && scrubOperation >= shownPath.operations.length)) scrubOperation = null;
      if (!same) scrubMoves = null;
      drawScrubbed();
    } catch (error) {
      shownPath = null;
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
  /** A post definition from the post editor, used instead of `post`. */
  definition?: string;
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
  const { post, machine, definition } = chosenTarget;
  const which = Array.isArray(setup) ? { setups: setup } : { setup };
  const lift = Number(element<HTMLSelectElement>("prove-out").value) || null;
  const path = await save({
    title: text.saveGcode,
    defaultPath: lift === null ? `${name}.nc` : `${name}-air-cut-${lift}.nc`,
    filters: [{ name: text.gcodeFiles, extensions: ["nc", "tap", "ngc"] }],
  });
  if (!path) return;
  // Every check first; the report goes next to the program.
  const reportPath = path.replace(/\.[^./\\]*$/, "") + ".report.txt";
  const report = await request<Verification>({
    method: "verify",
    post,
    definition,
    machine,
    setups: Array.isArray(setup) ? setup : [setup],
    path: reportPath,
  });
  if (report.verdict === "problems") {
    const list = report.problems.map((p) => `• ${p}`).join("\n");
    const go = await ask(
      text.checksFound(report.problems.length, list, reportPath),
      { title: text.verificationProblems, kind: "warning", okLabel: text.saveAnyway, cancelLabel: text.dontSave },
    );
    if (!go) {
      showReport(report);
      return;
    }
  }
  // An override is recorded in the program's header (docs/safety.md, gate 4).
  const result = await request<{ gcode: string; notes: string[] }>({
    method: "post",
    post,
    definition,
    machine,
    ...which,
    path,
    override: report.verdict === "problems",
    ...(lift === null ? {} : { prove_out: lift }),
  });
  const lines = result.gcode.split("\n").length;
  if (lift !== null) {
    showError(new Error(text.savedProveOut(lines, lift)));
    return;
  }
  const verdict =
    report.verdict === "problems"
      ? text.verdictProblems
      : report.verdict === "review"
        ? text.verdictReview(report.review.length)
        : text.verdictClear;
  showError(new Error(text.savedGcode(lines, verdict)));
}

interface Verification {
  verdict: "problems" | "review" | "clear";
  problems: string[];
  review: string[];
  text: string;
}

/** Shows a verification report in its panel. */
function showReport(report: Verification): void {
  const panel = element("report-dialog");
  element("report-text").textContent = report.text;
  panel.dataset["verdict"] = report.verdict;
  panel.hidden = false;
}

/** The scrub controls under a setup whose toolpath is drawn: an operation and a move slider. */
function scrubRow(path: Toolpath): HTMLElement {
  const row = globalThis.document.createElement("div");
  row.className = "scrub-row";
  row.title = text.scrubTitle;
  const which = globalThis.document.createElement("select");
  const all = globalThis.document.createElement("option");
  all.value = "";
  all.textContent = text.scrubAll;
  which.append(all);
  path.operations.forEach((operation, index) => {
    const option = globalThis.document.createElement("option");
    option.value = String(index);
    option.textContent = `${index + 1}. ${operation.name}`;
    which.append(option);
  });
  which.value = scrubOperation === null ? "" : String(scrubOperation);
  const slider = globalThis.document.createElement("input");
  slider.type = "range";
  slider.min = "0";
  slider.setAttribute("aria-label", text.scrubSliderLabel);
  const label = globalThis.document.createElement("span");
  label.className = "scrub-count";
  const sync = (): void => {
    const [first, end] = operationRange(path, scrubOperation);
    const total = end - first;
    slider.max = String(total);
    const at = Math.min(scrubMoves ?? total, total);
    slider.value = String(at);
    label.textContent = text.scrubMoves(at, total);
  };
  sync();
  which.addEventListener("change", () => {
    scrubOperation = which.value === "" ? null : Number(which.value);
    scrubMoves = null;
    sync();
    drawScrubbed();
  });
  // Dragging only redraws the toolpath: rebuilding the list would drop the slider mid-drag.
  slider.addEventListener("input", () => {
    const total = Number(slider.max);
    scrubMoves = Number(slider.value) >= total ? null : Number(slider.value);
    label.textContent = text.scrubMoves(Number(slider.value), total);
    drawScrubbed();
  });
  row.append(which, slider, label);
  return row;
}

function renderSetups(state: DocumentState): void {
  const setups = state.document.cam?.setups ?? [];
  ui.setupList.replaceChildren();
  ui.noSetups.hidden = setups.length > 0;
  element<HTMLButtonElement>("cam-post-all").disabled = setups.length < 2;
  element<HTMLButtonElement>("cam-setup-sheet").disabled = setups.length < 1;
  for (const { id, setup } of setups) {
    const item = globalThis.document.createElement("li");
    item.classList.toggle("showing", shownSetup === id);
    const head = globalThis.document.createElement("div");
    head.className = "head";
    const name = globalThis.document.createElement("span");
    name.className = "name";
    const table = setup.index === undefined ? "" : text.tableAngle(setup.index);
    name.textContent = text.setupName(setup.name, 53 + setup.work_offset, table);
    const show = globalThis.document.createElement("button");
    show.type = "button";
    show.className = "secondary";
    show.textContent = shownSetup === id ? text.hide : text.show;
    show.addEventListener("click", guarded(() => showToolpath(shownSetup === id ? null : id)));
    const post = globalThis.document.createElement("button");
    post.type = "button";
    post.className = "secondary";
    post.textContent = text.post;
    post.addEventListener("click", guarded(() => postSetup(id, `${state.document.name} ${setup.name}`)));
    const controller = globalThis.document.createElement("select");
    controller.title = text.controllerTitle;
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
    remove.title = text.deleteSetup;
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
      li.title = text.doubleClickToDelete;
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
      ["face", text.addFace],
      ["contour", text.addContour],
      ["pocket", text.addPocket],
      ["holes", text.addHoles],
      ["chamfer", text.addChamfer],
    ]) {
      const option = globalThis.document.createElement("option");
      option.value = value ?? "";
      option.textContent = label ?? "";
      kind.append(option);
    }
    const add = globalThis.document.createElement("button");
    add.type = "button";
    add.className = "secondary";
    add.textContent = text.add;
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
    detect.textContent = text.detect;
    detect.title = text.detectTitle;
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
            ? text.nothingNew
            : text.addedOperations(found.operations.length);
        if (found.notes.length > 0 || found.operations.length === 0) {
          showError(new Error(`${added} ${found.notes.join(" ")}`.trim()));
        }
      }),
    );
    const simulate = globalThis.document.createElement("button");
    simulate.type = "button";
    simulate.className = "secondary";
    simulate.textContent = simulatedSetup === id ? text.hideStock : text.simulate;
    simulate.title = text.simulateTitle;
    simulate.addEventListener(
      "click",
      guarded(() => simulateSetup(simulatedSetup === id ? null : id)),
    );
    row.append(kind, add, detect, simulate);
    // The vise holding the stock (decision 0020): how much it grips, from
    // the stock's bottom up; empty for none. Its jaws are checked against
    // every tool in the simulation.
    const viseRow = globalThis.document.createElement("label");
    viseRow.className = "vise-row";
    viseRow.title = text.viseTitle;
    const vise = globalThis.document.createElement("input");
    vise.type = "number";
    vise.min = "0";
    vise.step = "any";
    vise.placeholder = text.noVise;
    vise.value = setup.vise ? String(setup.vise.grip) : "";
    const along = globalThis.document.createElement("select");
    for (const [value, label] of [
      ["x", text.jawsFrontBack],
      ["y", text.jawsLeftRight],
    ] as const) {
      const option = globalThis.document.createElement("option");
      option.value = value;
      option.textContent = label;
      along.append(option);
    }
    along.value = setup.vise?.along ?? "x";
    const applyVise = guarded(async () => {
      const grip = vise.value.trim() === "" ? null : Number(vise.value);
      render(
        await execute(
          "cam.set_vise",
          grip === null ? { setup: id } : { setup: id, grip, along: along.value, jaw: setup.vise?.jaw ?? 25 },
        ),
      );
    });
    vise.addEventListener("change", applyVise);
    along.addEventListener("change", applyVise);
    viseRow.append(text.viseLabel, vise, along);
    item.append(head, ops);
    if (shownSetup === id && shownPath) item.append(scrubRow(shownPath));
    item.append(row, viseRow);
    ui.setupList.append(item);
  }
}

/** One line about a feature for the timeline. */
function featureDetail(feature: import("./api").Feature, id?: number): string {
  if (feature.type === "extrude") {
    const fx = feature.distance_expression ? "ƒ " : "";
    return text.extrudeDetail(fx, feature.operation === "cut", formatNumber(feature.distance));
  }
  if (feature.type === "revolve") {
    const fx = feature.angle_expression ? "ƒ " : "";
    return text.revolveDetail(fx, feature.operation === "cut", formatNumber(feature.angle));
  }
  if (feature.type === "chamfer") return text.chamferDetail(formatNumber(feature.distance));
  if (feature.type === "pattern") {
    const source = current?.document.features?.find((f) => f.id === feature.feature)?.feature.name ?? "?";
    return text.patternDetail(feature.count_x, feature.count_y, source);
  }
  if (feature.type === "circular_pattern") {
    const source = current?.document.features?.find((f) => f.id === feature.feature)?.feature.name ?? "?";
    return text.circularDetail(feature.count, formatNumber(feature.angle), source);
  }
  if (feature.type === "fillet") return text.filletDetail(formatNumber(feature.radius));
  if (feature.type === "reference") {
    const triangles = id === undefined ? undefined : current?.references?.[String(id)]?.triangles;
    return text.referenceDetail(feature.source, triangles);
  }
  if (feature.type === "import") {
    const size =
      feature.bytes >= 1048576
        ? text.megabytes(formatNumber(feature.bytes / 1048576))
        : text.kilobytes(Math.ceil(feature.bytes / 1024));
    return text.importDetail(feature.operation === "cut", feature.source, size);
  }
  const { spec, points } = feature;
  const size = spec.thread ? spec.thread.size.replace(/x.*/, "") : `Ø${formatNumber(spec.diameter)}`;
  const depth = spec.depth === "through" ? text.thru : text.deep(formatNumber(spec.depth.blind));
  return text.holeDetail(points.length, size, Boolean(spec.counterbore), depth);
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
  closeRevolve();
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
    ui.patternTitle.textContent = text.edit(existing.name);
    ui.patternSource.value = String(existing.feature);
    ui.patternSource.disabled = true;
    ui.patternCountX.value = String(existing.count_x);
    ui.patternSpacingX.value = String(existing.spacing_x);
    ui.patternCountY.value = String(existing.count_y);
    ui.patternSpacingY.value = String(existing.spacing_y);
  } else {
    ui.patternTitle.textContent = text.pattern;
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
  ui.circularDialog.hidden = true;
  editingCircular = null;
}

let editingCircular: number | null = null;

/** The centre choices for a source feature: the sketch origin and the points of its sketch. */
function fillCircularCenters(source: number, chosen: number | undefined): void {
  if (!current) return;
  const features = current.document.features ?? [];
  const feature = features.find((f) => f.id === source)?.feature;
  const sketchId =
    feature && (feature.type === "extrude" || feature.type === "hole" || feature.type === "revolve")
      ? feature.sketch
      : undefined;
  const entities = sketchId === undefined ? {} : (current.document.sketches?.[String(sketchId)]?.sketch.entities ?? {});
  const origin = globalThis.document.createElement("option");
  origin.value = "";
  origin.textContent = text.sketchOrigin;
  const options = Object.entries(entities)
    .filter(([, e]) => e.geometry.type === "point")
    .map(([key, e]) => {
      const option = globalThis.document.createElement("option");
      option.value = key;
      const g = e.geometry;
      option.textContent =
        g.type === "point" ? text.pointLabel(Number(key), formatNumber(g.at.x), formatNumber(g.at.y)) : key;
      return option;
    });
  ui.circularCenter.replaceChildren(origin, ...options);
  ui.circularCenter.value = chosen === undefined ? "" : String(chosen);
}

function openCircular(feature: number | null): void {
  if (!current) return;
  closeExtrude();
  closeRevolve();
  closeHole();
  closeBlend();
  closePattern();
  editingCircular = feature;
  const features = current.document.features ?? [];
  ui.circularSource.replaceChildren(
    ...features
      .filter((f) => f.feature.type === "extrude" || f.feature.type === "hole" || f.feature.type === "revolve")
      .map((f) => {
        const option = globalThis.document.createElement("option");
        option.value = String(f.id);
        option.textContent = f.feature.name;
        return option;
      }),
  );
  const existing = features.find((f) => f.id === feature)?.feature;
  if (existing?.type === "circular_pattern") {
    ui.circularTitle.textContent = text.edit(existing.name);
    ui.circularSource.value = String(existing.feature);
    ui.circularSource.disabled = true;
    ui.circularCount.value = String(existing.count);
    ui.circularAngle.value = String(existing.angle);
    fillCircularCenters(existing.feature, existing.center);
  } else {
    ui.circularTitle.textContent = text.circularPattern;
    ui.circularSource.disabled = false;
    const last = ui.circularSource.options[ui.circularSource.options.length - 1];
    if (last) ui.circularSource.value = last.value;
    fillCircularCenters(Number(ui.circularSource.value), undefined);
  }
  ui.circularDialog.hidden = false;
  ui.circularSource.focus();
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
    faces ? text.faceCount(faces) : "",
    edges ? text.edgeCount(edges) : "",
  ]
    .filter(Boolean)
    .join(text.and);
  ui.blendPicked.textContent =
    blending.editing !== null
      ? text.edgesStay
      : picked === ""
        ? text.pickEdges
        : text.pickedMore(picked);
  viewer.highlightFaces(blending.faces);
  viewer.highlightEdges(blending.edges);
}

function openBlend(kind: "chamfer" | "fillet", feature: number | null): void {
  if (!current) return;
  closeExtrude();
  closeRevolve();
  closeHole();
  const existing = current.document.features?.find((f) => f.id === feature)?.feature;
  blending = { kind, faces: [], edges: [], editing: feature };
  const label = kind === "chamfer" ? text.chamfer : text.fillet;
  ui.blendTitle.textContent = existing ? text.edit(existing.name) : label;
  ui.blendSizeLabel.textContent = kind === "chamfer" ? text.distanceMm : text.radiusMm;
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
      ? text.holeKeepsPoints
      : count === 0
        ? text.noFreePoints
        : text.holesAtPoints(count);
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
  closeRevolve();
  editingHole = feature;
  const sketches = Object.entries(current.document.sketches ?? {});
  ui.holeSketch.replaceChildren(
    ...sketches.map(([id, record]) => {
      const option = globalThis.document.createElement("option");
      option.value = id;
      option.textContent = text.sketchWithPoints(record.name, planeLabel(record.plane), freePoints(id, current!).length);
      return option;
    }),
  );
  const existing = current.document.features?.find((f) => f.id === feature)?.feature;
  if (existing?.type === "hole") {
    const { spec } = existing;
    ui.holeTitle.textContent = text.edit(existing.name);
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
    ui.holeTitle.textContent = text.hole;
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
      option.textContent = text.sketchInPlane(record.name, planeLabel(record.plane));
      return option;
    }),
  );
  const existing = current.document.features?.find((f) => f.id === feature)?.feature;
  if (existing?.type === "extrude") {
    ui.extrudeTitle.textContent = text.edit(existing.name);
    ui.extrudeSketch.value = String(existing.sketch);
    ui.extrudeSketch.disabled = true;
    ui.extrudeDistance.value = existing.distance_expression ?? String(existing.distance);
    ui.extrudeOperation.value = existing.operation;
  } else {
    ui.extrudeTitle.textContent = text.extrude;
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

let editingRevolve: number | null = null;

/** The axis choices for a sketch: its own two axes, then each of its lines. */
function fillRevolveAxes(sketch: string): void {
  const options: [string, string][] = [
    ["y", text.axisY],
    ["x", text.axisX],
  ];
  const entities = current?.document.sketches?.[sketch]?.sketch.entities ?? {};
  for (const [id, entity] of Object.entries(entities)) {
    if (entity.geometry.type === "line") options.push([`line:${id}`, text.axisLine(Number(id))]);
  }
  ui.revolveAxis.replaceChildren(
    ...options.map(([value, label]) => {
      const option = globalThis.document.createElement("option");
      option.value = value;
      option.textContent = label;
      return option;
    }),
  );
}

function openRevolve(feature: number | null): void {
  if (!current) return;
  closeExtrude();
  closeHole();
  closeBlend();
  closePattern();
  if (editor.editing !== null) {
    editor.close();
    updateSketchUi();
  }
  editingRevolve = feature;
  const sketches = Object.entries(current.document.sketches ?? {});
  ui.revolveSketch.replaceChildren(
    ...sketches.map(([id, record]) => {
      const option = globalThis.document.createElement("option");
      option.value = id;
      option.textContent = text.sketchInPlane(record.name, planeLabel(record.plane));
      return option;
    }),
  );
  const existing = current.document.features?.find((f) => f.id === feature)?.feature;
  if (existing?.type === "revolve") {
    ui.revolveTitle.textContent = text.edit(existing.name);
    ui.revolveSketch.value = String(existing.sketch);
    ui.revolveSketch.disabled = true;
    fillRevolveAxes(String(existing.sketch));
    ui.revolveAxis.value =
      existing.axis.type === "line" ? `line:${existing.axis.entity}` : existing.axis.type === "sketch_x" ? "x" : "y";
    ui.revolveAngle.value = existing.angle_expression ?? String(existing.angle);
    ui.revolveOperation.value = existing.operation;
  } else {
    ui.revolveTitle.textContent = text.revolve;
    ui.revolveSketch.disabled = false;
    const last = sketches.at(-1);
    if (last) ui.revolveSketch.value = last[0];
    fillRevolveAxes(ui.revolveSketch.value);
    ui.revolveAngle.value = "360";
    ui.revolveOperation.value = (current.document.features?.length ?? 0) === 0 ? "join" : ui.revolveOperation.value;
  }
  ui.revolveDialog.hidden = false;
  ui.revolveAngle.focus();
  ui.revolveAngle.select();
}

function closeRevolve(): void {
  ui.revolveDialog.hidden = true;
  editingRevolve = null;
}

function updateSketchUi(): void {
  const editing = editor.editing;
  if (editing !== null) {
    closeExtrude();
  closeRevolve();
    closeHole();
    closeBlend();
    closePattern();
  }
  // The SKETCH tab appears while a sketch is open, and takes over the ribbon.
  const sketchTab = globalThis.document.querySelector<HTMLButtonElement>('.ribbon-tab[data-tab="sketch"]');
  if (sketchTab) sketchTab.hidden = editing === null;
  if (editing !== null) showTab("sketch");
  else if (activeTab === "sketch") showTab(workspace === "design" ? "solid" : "milling");
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
  const name = current.document.sketches?.[id]?.name ?? text.sketch;
  ui.statusSketch.textContent = `${name}: ${freedomText(id, current)}`;
  ui.statusSketch.hidden = false;
}

/** Asks before throwing away unsaved changes. Returns true to go ahead. */
async function confirmDiscard(action: string): Promise<boolean> {
  if (!current?.dirty) return true;
  return ask(text.unsavedQuestion(action), {
    title: text.unsaved,
    kind: "warning",
    okLabel: text.discardChanges,
    cancelLabel: text.cancel,
  });
}

async function saveAs(): Promise<boolean> {
  const path = await save({
    title: text.saveProject,
    defaultPath: `${current?.document.name ?? text.untitled}.cndt`,
    filters: [{ name: text.cndtProject, extensions: ["cndt"] }],
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
    [text.file, path.split(/[\\/]/).pop() ?? path],
    [text.solids, String(info.solids)],
    [text.faces, String(info.faces)],
    [text.edges, String(info.edges)],
    [text.vertices, String(info.vertices)],
  ];
  if (info.bounds) {
    const { min, max } = info.bounds;
    const size = [max.x - min.x, max.y - min.y, max.z - min.z].map((v) => v.toFixed(2));
    rows.push([text.size, text.mmSize(size)]);
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
    if (!(await confirmDiscard(text.startNewDocument))) return;
    render(
      await request<DocumentState>({
        method: "new_document",
        name: text.untitled,
        discard_changes: true,
      }),
    );
  }),
);

/** Shows the picture saved in a project file and asks to go ahead. True to open it. */
async function previewProject(path: string): Promise<boolean> {
  const { png_base64: png } = await request<ProjectThumbnail>({ method: "project_thumbnail", path });
  const panel = element("open-preview");
  const image = element<HTMLImageElement>("open-preview-image");
  image.hidden = !png;
  if (png) image.src = `data:image/png;base64,${png}`;
  element("open-preview-note").textContent = png ? text.previewShown : text.previewNone;
  element("open-preview-name").textContent = path.split(/[\\/]/).pop() ?? path;
  panel.hidden = false;
  const okButton = element("open-preview-ok");
  okButton.focus();
  return new Promise<boolean>((resolve) => {
    const finish = (answer: boolean): void => {
      panel.hidden = true;
      okButton.removeEventListener("click", onOk);
      element("open-preview-cancel").removeEventListener("click", onCancel);
      window.removeEventListener("keydown", onKey);
      resolve(answer);
    };
    const onOk = (): void => finish(true);
    const onCancel = (): void => finish(false);
    const onKey = (event: KeyboardEvent): void => {
      if (event.key === "Escape") finish(false);
    };
    okButton.addEventListener("click", onOk);
    element("open-preview-cancel").addEventListener("click", onCancel);
    window.addEventListener("keydown", onKey);
  });
}

element("open").addEventListener(
  "click",
  guarded(async () => {
    const path = await open({
      title: text.openProject,
      multiple: false,
      directory: false,
      filters: [{ name: text.cndtProject, extensions: ["cndt"] }],
    });
    if (!path) return;
    if (!(await previewProject(path))) return;
    if (!(await confirmDiscard(text.openAnotherProject))) return;
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

// A STEP solid into the design: a timeline feature, its file kept in the
// project (decision 0017).
element("import-step").addEventListener(
  "click",
  guarded(async () => {
    const path = await open({
      title: text.importStepTitle,
      multiple: false,
      directory: false,
      filters: [{ name: text.stepOrIges, extensions: ["step", "stp", "STEP", "STP", "igs", "iges", "IGS", "IGES"] }],
    });
    if (typeof path !== "string") return;
    ui.viewportHint.textContent = text.readingFile;
    ui.viewportHint.hidden = false;
    const next = await request<DocumentState>({ method: "import_step", path });
    render(next);
    viewer.fit();
  }),
);

// An STL mesh beside the part, for reference and measuring (decision 0018).
element("import-mesh").addEventListener(
  "click",
  guarded(async () => {
    const path = await open({
      title: text.importMeshTitle,
      multiple: false,
      directory: false,
      filters: [{ name: text.stlFiles, extensions: ["stl", "STL"] }],
    });
    if (typeof path !== "string") return;
    render(await request<DocumentState>({ method: "import_mesh", path }));
    await referencesShown;
    viewer.fit();
  }),
);

// A STEP file shown on its own, not added to the design.
element("preview-step").addEventListener(
  "click",
  guarded(async () => {
    const path = await open({
      title: text.lookAtStepTitle,
      multiple: false,
      directory: false,
      filters: [{ name: text.stepFiles, extensions: ["step", "stp", "STEP", "STP"] }],
    });
    if (!path) return;
    ui.viewportHint.textContent = text.readingFile;
    ui.viewportHint.hidden = false;
    try {
      const info = await request<StepInfo>({ method: "step_info", path });
      viewer.show(await stepPreview(path));
      showModel(info, path);
    } catch (error) {
      ui.viewportHint.textContent = text.importStepHint;
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
  closeRevolve();
    render(next);
  })();
});

ui.revolveButton.addEventListener("click", () => openRevolve(null));
ui.revolveSketch.addEventListener("change", () => fillRevolveAxes(ui.revolveSketch.value));
element("revolve-cancel").addEventListener("click", closeRevolve);
ui.revolveDialog.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeRevolve();
});
ui.revolveDialog.addEventListener("submit", (event) => {
  event.preventDefault();
  guarded(async () => {
    // A number of degrees, or an expression of the parameters.
    const entered = ui.revolveAngle.value.trim();
    const angle = Number.isFinite(Number(entered)) && entered !== "" ? Number(entered) : entered;
    const operation = ui.revolveOperation.value;
    const axis = ui.revolveAxis.value;
    const turn = axis.startsWith("line:") ? { axis_line: Number(axis.slice(5)) } : { axis };
    const next =
      editingRevolve === null
        ? await execute("feature.revolve", { sketch: Number(ui.revolveSketch.value), angle, operation, ...turn })
        : await execute("feature.edit_revolve", { feature: editingRevolve, angle, operation, ...turn });
    closeRevolve();
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
    showError(new Error(text.noPartYet));
    return;
  }
  pickingFace = true;
  viewer.canvas.style.cursor = "pointer";
  ui.viewportHint.textContent = text.clickFlatFace;
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
element("cam-add-side").addEventListener(
  "click",
  guarded(async () => {
    if (!current?.document.cam?.stock) render(await execute("cam.set_stock", { side: 2, top: 2, bottom: 3 }));
    const setups = current?.document.cam?.setups?.length ?? 0;
    const table = element<HTMLSelectElement>("cam-turn").value === "table";
    const side = element<HTMLSelectElement>("cam-side").value;
    // The B table turns the part about its upright axis, model Y, so it
    // reaches the left and right sides; the front and back are turned by
    // hand. B90 for the right is assumed, and every program says to check it.
    const angle = side === "right" ? 90 : side === "left" ? 270 : null;
    const turn = table && angle !== null ? { index: angle } : {};
    render(await execute("cam.add_setup", { side, work_offset: Math.min(setups + 1, 6), ...turn }));
  }),
);
element("cam-post-all").addEventListener(
  "click",
  guarded(async () => {
    const setups = current?.document.cam?.setups ?? [];
    await postSetup(
      setups.map((s) => s.id),
      text.allSides(current?.document.name ?? text.part),
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
    if (!name) throw new Error(text.faceNoName);
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
ui.circularButton.addEventListener("click", () => openCircular(null));
element("circular-cancel").addEventListener("click", closePattern);
ui.circularSource.addEventListener("change", () =>
  fillCircularCenters(Number(ui.circularSource.value), undefined),
);
ui.circularDialog.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closePattern();
});
ui.circularDialog.addEventListener("submit", (event) => {
  event.preventDefault();
  guarded(async () => {
    const args = {
      count: Number(ui.circularCount.value),
      angle: Number(ui.circularAngle.value),
      ...(ui.circularCenter.value === "" ? {} : { center: Number(ui.circularCenter.value) }),
    };
    const next =
      editingCircular === null
        ? await execute("feature.circular_pattern", { feature: Number(ui.circularSource.value), ...args })
        : await execute("feature.edit_circular_pattern", { feature: editingCircular, ...args });
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
        throw new Error(text.pickOne);
      }
      const faces = blending.faces.map((f) => {
        const name = uniqueFaceName(f);
        if (!name) throw new Error(text.pickedFaceNoName);
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
          throw new Error(text.pickedEdgeNoName);
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
    if (!name) throw new Error(text.faceNoNameForSketch);
    const next = await execute("sketch.create", { face: name });
    const ids = Object.keys(next.document.sketches ?? {}).map(Number);
    const created = Math.max(...ids);
    const frame = next.sketch_frames[String(created)];
    if (frame?.state === "failed") {
      // Not a flat face: take the sketch back out and say why.
      render(await request<DocumentState>({ method: "undo" }));
      throw new Error(text.cantSketchThere(frame.reason));
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
    if (!(await confirmDiscard(text.close))) event.preventDefault();
  })
  .catch(showError);

state().then(render).catch(showError);

// ---------------------------------------------------------------- window layout

/** Which workspace is showing: modelling, or programming the machine. */
let workspace: "design" | "manufacture" = "design";
/** Which ribbon tab is showing. */
let activeTab = "solid";

function showTab(tab: string): void {
  activeTab = tab;
  for (const button of globalThis.document.querySelectorAll<HTMLButtonElement>(".ribbon-tab")) {
    button.classList.toggle("active", button.dataset["tab"] === tab);
    button.setAttribute("aria-selected", String(button.dataset["tab"] === tab));
  }
  for (const pane of globalThis.document.querySelectorAll<HTMLElement>(".ribbon-pane")) {
    pane.hidden = pane.dataset["pane"] !== tab;
  }
}

function showWorkspace(next: "design" | "manufacture"): void {
  workspace = next;
  element<HTMLSelectElement>("workspace").value = next;
  globalThis.document.body.dataset["workspace"] = next;
  for (const tab of globalThis.document.querySelectorAll<HTMLButtonElement>(".ribbon-tab[data-workspace]")) {
    tab.hidden = tab.dataset["workspace"] !== next;
  }
  for (const node of globalThis.document.querySelectorAll<HTMLElement>(".tree [data-workspace]")) {
    node.hidden = node.dataset["workspace"] !== next;
  }
  if (activeTab !== "sketch") showTab(next === "design" ? "solid" : "milling");
  // Programming wants the setups in view; modelling, the sketches.
  const sketches = element("sketch-list").closest("details");
  const setups = element("setup-list").closest("details");
  if (sketches) sketches.open = next === "design";
  if (setups) setups.open = next === "manufacture";
}

element<HTMLSelectElement>("workspace").addEventListener("change", (event) => {
  const value = (event.target as HTMLSelectElement).value;
  showWorkspace(value === "manufacture" ? "manufacture" : "design");
});
for (const tab of globalThis.document.querySelectorAll<HTMLButtonElement>(".ribbon-tab")) {
  tab.addEventListener("click", () => showTab(tab.dataset["tab"] ?? "solid"));
}
showWorkspace("design");

/** Opens a small menu under `anchor`; any click elsewhere closes it. */
function popup(anchor: HTMLElement, items: { label: string; title: string; run: () => void; disabled: boolean }[]): void {
  globalThis.document.querySelector(".popup-menu")?.remove();
  const menu = globalThis.document.createElement("div");
  menu.className = "menu popup-menu";
  menu.setAttribute("role", "menu");
  for (const item of items) {
    const button = globalThis.document.createElement("button");
    button.type = "button";
    button.setAttribute("role", "menuitem");
    button.textContent = item.label;
    button.title = item.title;
    button.disabled = item.disabled;
    button.addEventListener("click", () => {
      menu.remove();
      item.run();
    });
    menu.append(button);
  }
  const rect = anchor.getBoundingClientRect();
  menu.style.left = `${rect.left}px`;
  menu.style.top = `${rect.bottom + 4}px`;
  globalThis.document.body.append(menu);
  const close = (event: PointerEvent) => {
    if (!menu.contains(event.target as Node) && event.target !== anchor) {
      menu.remove();
      globalThis.document.removeEventListener("pointerdown", close);
    }
  };
  globalThis.document.addEventListener("pointerdown", close);
}

// A group's label opens a menu of everything in the group, by name.
for (const group of globalThis.document.querySelectorAll<HTMLElement>(".ribbon-group")) {
  const label = group.querySelector<HTMLButtonElement>(".ribbon-label");
  if (!label) continue;
  label.addEventListener("click", () => {
    const tools = [...group.querySelectorAll<HTMLButtonElement>(".ribbon-buttons > button, .ribbon-buttons .tool, .ribbon-buttons .chip")];
    const unique = [...new Set(tools)];
    popup(
      label,
      unique.map((tool) => ({
        label: tool.querySelector("span")?.textContent?.trim() || tool.textContent?.trim() || "",
        title: tool.title,
        disabled: tool.disabled,
        run: () => tool.click(),
      })),
    );
  });
}

// The File menu.
const fileButton = element<HTMLButtonElement>("file-menu-button");
const fileMenu = element<HTMLDivElement>("file-menu");
fileButton.addEventListener("click", () => {
  fileMenu.hidden = !fileMenu.hidden;
  fileButton.setAttribute("aria-expanded", String(!fileMenu.hidden));
});
fileMenu.addEventListener("click", () => {
  fileMenu.hidden = true;
  fileButton.setAttribute("aria-expanded", "false");
});
globalThis.document.addEventListener("pointerdown", (event) => {
  const target = event.target as Node;
  if (!fileMenu.hidden && !fileMenu.contains(target) && !fileButton.contains(target)) {
    fileMenu.hidden = true;
    fileButton.setAttribute("aria-expanded", "false");
  }
});
element("quick-save").addEventListener("click", () => element("save").click());
element("insert-step").addEventListener("click", () => element("import-step").click());

// Views and display.
for (const button of globalThis.document.querySelectorAll<HTMLButtonElement>("[data-view]")) {
  button.addEventListener("click", () => viewer.look(button.dataset["view"] as ViewName));
}
element("nav-fit").addEventListener("click", () => element("fit").click());
const wire = element<HTMLButtonElement>("nav-wire");
wire.addEventListener("click", () => {
  const on = wire.getAttribute("aria-pressed") !== "true";
  wire.setAttribute("aria-pressed", String(on));
  viewer.setWireframe(on);
});

// Layers: the eyes in the browser and the grid button.
for (const eye of globalThis.document.querySelectorAll<HTMLButtonElement>("[data-layer]")) {
  eye.addEventListener("click", (event) => {
    // An eye inside a tree heading must not fold the heading.
    event.preventDefault();
    event.stopPropagation();
    const layer = eye.dataset["layer"] as Layer;
    const on = !viewer.isShown(layer);
    viewer.setLayer(layer, on);
    eye.setAttribute("aria-pressed", String(on));
  });
}

// The browser: docked beside the view, floating over it, or hidden.
const stage = element("stage");
const dock = element<HTMLButtonElement>("browser-dock");
dock.addEventListener("click", () => {
  const floating = stage.classList.toggle("browser-floating");
  dock.setAttribute("aria-pressed", String(!floating));
});
element("browser-hide").addEventListener("click", () => {
  stage.classList.add("browser-hidden");
  element("browser-show").hidden = false;
});
element("browser-show").addEventListener("click", () => {
  stage.classList.remove("browser-hidden");
  element("browser-show").hidden = true;
});

// The timeline's playback buttons move the history marker.
const built = () => current?.rollback ?? current?.document.features?.length ?? 0;
element("tl-start").addEventListener("click", guarded(() => rollTo(0)));
element("tl-back").addEventListener("click", guarded(() => rollTo(built() - 1)));
element("tl-forward").addEventListener("click", guarded(() => rollTo(built() + 1)));
element("tl-end").addEventListener("click", guarded(() => rollTo(Number.MAX_SAFE_INTEGER)));

// Simulate from the ribbon: the setup whose toolpath is shown, or the first.
element("cam-simulate").addEventListener(
  "click",
  guarded(async () => {
    const setups = current?.document.cam?.setups ?? [];
    const target = shownSetup ?? setups[0]?.id;
    if (target === undefined) throw new Error(text.addSetupFirst);
    await simulateSetup(target);
  }),
);

// Verify from the ribbon: every setup as one program, shown, not saved.
element("cam-verify").addEventListener(
  "click",
  guarded(async () => {
    const setups = (current?.document.cam?.setups ?? []).map((s) => s.id);
    if (setups.length === 0) throw new Error(text.addSetupFirst);
    const { post, machine, definition } = chosenTarget;
    showReport(await request<Verification>({ method: "verify", post, definition, machine, setups }));
  }),
);
element("report-close").addEventListener("click", () => {
  element("report-dialog").hidden = true;
});

/** The tool library in the browser, with each tool's assembly. */
function renderTools(state: DocumentState): void {
  const list = element("tool-list");
  list.replaceChildren();
  for (const [number, tool] of Object.entries(state.document.cam?.tools ?? {})) {
    const a = state.tool_assemblies?.[number];
    const item = globalThis.document.createElement("li");
    const name = globalThis.document.createElement("span");
    name.className = "name";
    name.textContent = text.toolName(number, tool.description);
    const detail = globalThis.document.createElement("span");
    detail.className = "detail";
    if (a) {
      detail.textContent = text.stickoutDetail(formatNumber(a.stickout));
      item.title = text.toolTitle(
        formatNumber(a.flute_length),
        formatNumber(a.stickout),
        formatNumber(a.shank_diameter),
        formatNumber(a.holder_diameter),
        formatNumber(a.holder_length),
        a.measured,
      );
      item.classList.toggle("typical", !a.measured);
    }
    item.append(name, detail);
    item.tabIndex = 0;
    item.addEventListener("click", () => openTool(Number(number)));
    item.addEventListener("keydown", (event) => {
      if (event.key === "Enter") openTool(Number(number));
    });
    list.append(item);
  }
}

// ---------------------------------------------------------------- tool editor

const toolDialog = element<HTMLFormElement>("tool-dialog");
const toolField = (id: string) => element<HTMLInputElement>(id);
const ASSEMBLY_FIELDS = [
  ["tool-flute", "flute_length"],
  ["tool-stickout", "stickout"],
  ["tool-shank", "shank_diameter"],
  ["tool-holder-diameter", "holder_diameter"],
  ["tool-holder-length", "holder_length"],
] as const;
let editingTool: number | null = null;

/** Shows the thread only for a tap, and the assembly only when measured. */
function syncToolFields(): void {
  const kind = element<HTMLSelectElement>("tool-kind").value;
  for (const row of toolDialog.querySelectorAll<HTMLElement>("[data-tool-for]")) {
    row.hidden = row.dataset["toolFor"] !== kind;
  }
  const measured = toolField("tool-measured").checked;
  for (const row of toolDialog.querySelectorAll<HTMLElement>("[data-tool-measured]")) row.hidden = !measured;
}

/** Opens the tool editor on a library tool, or on a new one for `null`. */
function openTool(number: number | null): void {
  if (!current) return;
  const tools = current.document.cam?.tools ?? {};
  const tool = number === null ? undefined : (tools[String(number)] as Record<string, unknown> | undefined);
  editingTool = number;
  element("tool-dialog-title").textContent = text.toolDialogTitle(number);
  const used = Object.keys(tools).map(Number);
  toolField("tool-number").value = String(number ?? Math.max(0, ...used) + 1);
  const kind = (tool?.["kind"] as { type: string; thread?: string } | undefined) ?? { type: "end_mill" };
  element<HTMLSelectElement>("tool-kind").value = kind.type;
  if (kind.thread) element<HTMLSelectElement>("tool-thread").value = kind.thread;
  toolField("tool-diameter").value = String(tool?.["diameter"] ?? 10);
  toolField("tool-rpm").value = String(tool?.["rpm"] ?? 8000);
  toolField("tool-feed").value = String(tool?.["feed"] ?? 1000);
  toolField("tool-description").value = String(tool?.["description"] ?? "");
  // The assembly the checks use now: measured, or typical for the kind.
  const assembly = number === null ? undefined : current.tool_assemblies?.[String(number)];
  toolField("tool-measured").checked = assembly?.measured ?? false;
  for (const [id, key] of ASSEMBLY_FIELDS) toolField(id).value = assembly ? String(assembly[key]) : "";
  element("tool-typical").textContent =
    assembly && !assembly.measured
      ? text.typicalSizes(
          formatNumber(assembly.flute_length),
          formatNumber(assembly.stickout),
          formatNumber(assembly.shank_diameter),
          formatNumber(assembly.holder_diameter),
          formatNumber(assembly.holder_length),
        )
      : "";
  element<HTMLButtonElement>("tool-delete").hidden = number === null;
  syncToolFields();
  toolDialog.hidden = false;
  toolField("tool-diameter").focus();
}

function closeTool(): void {
  toolDialog.hidden = true;
  editingTool = null;
}

element("tool-add").addEventListener("click", () => openTool(null));
element("tool-cancel").addEventListener("click", closeTool);
element("tool-kind").addEventListener("change", syncToolFields);
toolField("tool-measured").addEventListener("change", syncToolFields);
toolDialog.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeTool();
});
element("tool-delete").addEventListener(
  "click",
  guarded(async () => {
    if (editingTool === null) return;
    render(await execute("cam.remove_tool", { number: editingTool }));
    closeTool();
  }),
);
toolDialog.addEventListener("submit", (event) => {
  event.preventDefault();
  guarded(async () => {
    const number = Number(toolField("tool-number").value);
    const kind = element<HTMLSelectElement>("tool-kind").value;
    const args: Record<string, unknown> = {
      number,
      kind,
      diameter: Number(toolField("tool-diameter").value),
      rpm: Number(toolField("tool-rpm").value),
      feed: Number(toolField("tool-feed").value),
      description: toolField("tool-description").value,
    };
    if (kind === "tap") args["thread"] = element<HTMLSelectElement>("tool-thread").value;
    if (toolField("tool-measured").checked) {
      for (const [id, key] of ASSEMBLY_FIELDS) args[key] = Number(toolField(id).value);
    }
    // A new number that another tool already has replaces it, once asked.
    const taken = number !== editingTool && current?.document.cam?.tools?.[String(number)] !== undefined;
    if (taken && !(await ask(text.replaceTool(number), { kind: "warning" }))) return;
    let next = await execute("cam.set_tool", args);
    // Renumbered: the old number goes.
    if (editingTool !== null && editingTool !== number) {
      next = await execute("cam.remove_tool", { number: editingTool });
    }
    render(next);
    closeTool();
  })();
});

// ---------------------------------------------------------------- post editor

const editorPanel = element("post-editor");
const editorText = element<HTMLTextAreaElement>("post-editor-text");
const editorPreview = element("post-editor-preview");
const editorStatus = element("post-editor-status");
const editorBase = element<HTMLSelectElement>("post-editor-base");
let previewTimer: number | undefined;

/** Shows the setup posted through the definition being edited, the lines
 * that differ from the shipped post marked. */
async function refreshPostPreview(): Promise<void> {
  const setups = current?.document.cam?.setups ?? [];
  const setup = shownSetup ?? setups[0]?.id;
  if (setup === undefined) {
    editorStatus.textContent = text.addSetupForProgram;
    editorPreview.replaceChildren();
    return;
  }
  try {
    const preview = await request<{ gcode: string; changed: number[] | null; base: string | null }>({
      method: "post_preview",
      definition: editorText.value,
      setup,
    });
    const changed = new Set(preview.changed ?? []);
    const lines = preview.gcode.replace(/\n$/, "").split("\n");
    editorPreview.replaceChildren(
      ...lines.map((line, i) => {
        const row = globalThis.document.createElement("div");
        row.textContent = line || " ";
        if (changed.has(i)) row.className = "changed";
        return row;
      }),
    );
    const count = changed.size;
    editorStatus.classList.remove("error");
    editorStatus.textContent = preview.base
      ? count === 0
        ? text.sameAsShipped(preview.base)
        : text.differsFromShipped(count, preview.base)
      : text.notShipped;
  } catch (error) {
    editorStatus.classList.add("error");
    editorStatus.textContent = error instanceof Error ? error.message : String(error);
  }
}

async function loadBase(id: string): Promise<void> {
  const source = await request<{ text: string }>({ method: "post_source", post: id });
  editorText.value = source.text;
  await refreshPostPreview();
}

element("cam-post-editor").addEventListener(
  "click",
  guarded(async () => {
    editorPanel.hidden = false;
    if (editorText.value === "") {
      editorBase.value = chosenTarget.post;
      await loadBase(chosenTarget.post);
    } else {
      await refreshPostPreview();
    }
  }),
);
element("post-editor-close").addEventListener("click", () => {
  editorPanel.hidden = true;
});
editorBase.addEventListener(
  "change",
  guarded(() => loadBase(editorBase.value)),
);
editorText.addEventListener("input", () => {
  window.clearTimeout(previewTimer);
  previewTimer = window.setTimeout(() => void refreshPostPreview(), 400);
});
element("post-editor-load").addEventListener(
  "click",
  guarded(async () => {
    const path = await open({ title: text.openPost, filters: [{ name: text.postFiles, extensions: ["toml"] }] });
    if (typeof path !== "string") return;
    editorText.value = (await request<{ text: string }>({ method: "read_post", path })).text;
    await refreshPostPreview();
  }),
);
element("post-editor-save").addEventListener(
  "click",
  guarded(async () => {
    const path = await save({ title: text.savePost, defaultPath: "my-post.toml", filters: [{ name: text.postFiles, extensions: ["toml"] }] });
    if (!path) return;
    await request({ method: "write_post", definition: editorText.value, path });
    editorStatus.textContent = text.savedTo(path);
  }),
);
element("post-editor-use").addEventListener(
  "click",
  guarded(async () => {
    // Checked by the engine before it is offered for posting.
    await request({ method: "post_preview", definition: editorText.value });
    const id = /^\s*id\s*=\s*"([^"]*)"/m.exec(editorText.value)?.[1] ?? "custom";
    const custom: Target = { key: "edited", label: text.editedPost(id), post: id, definition: editorText.value };
    // Same post id as the chosen machine's: the machine still checks it.
    if (chosenTarget.post === id && chosenTarget.machine) custom.machine = chosenTarget.machine;
    const at = TARGETS.findIndex((t) => t.key === "edited");
    if (at >= 0) TARGETS.splice(at, 1);
    TARGETS.push(custom);
    chosenTarget = custom;
    if (current) renderSetups(current);
    editorStatus.textContent = text.usingEditedPost(custom.label);
  }),
);

// Export the part, as built at the history marker.
element("export").addEventListener(
  "click",
  guarded(async () => {
    const name = current?.document.name ?? text.part;
    const path = await save({
      title: text.exportPart,
      defaultPath: `${name}.step`,
      filters: [
        { name: text.stepFiles, extensions: ["step", "stp"] },
        { name: text.stlMesh, extensions: ["stl"] },
      ],
    });
    if (!path) return;
    const result = await request<{ format: string; triangles?: number }>({ method: "export", path });
    const what = result.format === "stl" ? text.anStlMesh(result.triangles ?? 0) : text.stepFiles;
    const rolled = current?.rollback !== null && current?.rollback !== undefined;
    showError(new Error(text.exportedPart(rolled, what, path)));
  }),
);

// DXF of a sketch, or of the part's top or bottom face, for 2D cutting.
element("export-dxf").addEventListener("click", () => {
  const exportDxf = async (choice: { sketch?: number; section?: number }, name: string) => {
    const path = await save({
      title: text.exportDxf,
      defaultPath: `${name}.dxf`,
      filters: [{ name: text.dxfFiles, extensions: ["dxf"] }],
    });
    if (!path) return;
    const result = await request<{ entities: number }>({ method: "export_dxf", path, ...choice });
    showError(new Error(text.exportedDxf(result.entities, path)));
  };
  const doc = current?.document;
  const items = [
    {
      label: text.profileLabel,
      title: text.profileTitle,
      disabled: !current?.body,
      run: guarded(() => {
        const bounds = current?.body?.bounds;
        const z = bounds ? (bounds.min.z + bounds.max.z) / 2 : 0;
        return exportDxf({ section: z }, text.profileName(doc?.name ?? text.part));
      }),
    },
    ...Object.entries(doc?.sketches ?? {}).map(([id, record]) => ({
      label: text.sketchLabel(record.name),
      title: text.sketchExportTitle,
      disabled: false,
      run: guarded(() => exportDxf({ sketch: Number(id) }, record.name)),
    })),
  ];
  popup(element("file-menu-button"), items);
});

// A DXF drawing into a new sketch on the XY plane.
element("import-dxf").addEventListener(
  "click",
  guarded(async () => {
    const path = await open({ title: text.importDxfTitle, filters: [{ name: text.dxfFiles, extensions: ["dxf"] }] });
    if (typeof path !== "string") return;
    const next = await request<DocumentState & { import_notes?: string[] }>({ method: "import_dxf", path });
    render(next);
    viewer.fit();
    const notes = next.import_notes ?? [];
    showError(new Error(text.importedDrawing(notes)));
  }),
);

// The setup sheet: a printable page for the operator at the machine.
element("cam-setup-sheet").addEventListener(
  "click",
  guarded(async () => {
    const path = await save({
      title: text.saveSetupSheet,
      defaultPath: `${current?.document.name ?? text.untitled} setup sheet.html`,
      filters: [{ name: text.setupSheetFiles, extensions: ["html"] }],
    });
    if (!path) return;
    await request({
      method: "setup_sheet",
      post: chosenTarget.post,
      definition: chosenTarget.definition,
      machine: chosenTarget.machine,
      path,
    });
    showError(new Error(text.savedSetupSheet(path)));
  }),
);

// The command palette (Ctrl+K or Cmd+K): every engine command and every button
// of the window, searched by name. A command that has a button runs that
// button; one without says what it takes.
interface PaletteItem extends PaletteEntry {
  button: HTMLButtonElement | null;
  note: string;
}

/** The button that stands for each engine command that has one. */
const commandButtons: Record<string, string> = {
  "feature.extrude": "extrude",
  "feature.edit_extrude": "extrude",
  "feature.revolve": "revolve",
  "feature.edit_revolve": "revolve",
  "feature.hole": "hole",
  "feature.edit_hole": "hole",
  "feature.pattern": "pattern",
  "feature.edit_pattern": "pattern",
  "feature.chamfer": "chamfer",
  "feature.fillet": "fillet",
  "feature.edit_blend": "chamfer",
  "feature.import": "insert-step",
  "sketch.create": "new-sketch",
  "cam.add_setup": "cam-add-top",
};

interface CommandSpec {
  name: string;
  description: string;
  args: { name: string; required: boolean }[];
}

const palette = element<HTMLDivElement>("palette");
const paletteInput = element<HTMLInputElement>("palette-input");
const paletteList = element<HTMLUListElement>("palette-list");
const paletteNote = element<HTMLParagraphElement>("palette-note");
let paletteItems: PaletteItem[] = [];
let paletteShown: PaletteItem[] = [];
let paletteIndex = 0;

function buttonLabel(button: HTMLButtonElement): string {
  const own = (button.querySelector("span")?.textContent ?? button.firstChild?.textContent ?? "").trim();
  if (own) return own;
  return (button.title || button.id).replace(/\s*\([^)]*\)\s*$/, "").trim();
}

async function buildPalette(): Promise<PaletteItem[]> {
  const specs = await request<CommandSpec[]>({ method: "commands" });
  const buttons = [
    ...globalThis.document.querySelectorAll<HTMLButtonElement>(".ribbon button.tool[id], .menu button[id]"),
  ];
  const items: PaletteItem[] = buttons.map((button) => ({
    label: buttonLabel(button),
    detail: button.title,
    keywords: specs.filter((spec) => commandButtons[spec.name] === button.id).map((spec) => spec.name),
    button,
    note: "",
  }));
  for (const spec of specs) {
    if (commandButtons[spec.name]) continue;
    const args = spec.args.map((arg) => (arg.required ? arg.name : `${arg.name}?`)).join(", ");
    items.push({
      label: spec.name,
      detail: firstSentence(spec.description),
      keywords: [],
      button: null,
      note: text.paletteNoForm(args || text.paletteNoArgs),
    });
  }
  return items;
}

function paletteRender(): void {
  paletteShown = matchEntries(paletteInput.value, paletteItems);
  paletteIndex = Math.min(paletteIndex, Math.max(0, paletteShown.length - 1));
  paletteList.replaceChildren();
  paletteShown.forEach((item, index) => {
    const row = globalThis.document.createElement("li");
    row.id = `palette-item-${index}`;
    row.setAttribute("role", "option");
    const name = globalThis.document.createElement("span");
    name.textContent = item.label;
    const detail = globalThis.document.createElement("span");
    detail.className = "palette-detail";
    detail.textContent = item.detail;
    row.append(name, detail);
    if (item.button?.disabled) row.setAttribute("aria-disabled", "true");
    row.addEventListener("click", () => paletteRun(item));
    paletteList.append(row);
  });
  paletteNote.textContent = "";
  paletteSelect(paletteIndex);
  if (paletteShown.length === 0) paletteNote.textContent = text.paletteNone;
}

function paletteSelect(index: number): void {
  paletteIndex = index;
  for (const [position, row] of [...paletteList.children].entries()) {
    row.setAttribute("aria-selected", String(position === index));
  }
  const item = paletteShown[index];
  paletteList.children[index]?.scrollIntoView({ block: "nearest" });
  paletteInput.setAttribute("aria-activedescendant", item ? `palette-item-${index}` : "");
  if (!item) return;
  paletteNote.textContent = item.button?.disabled ? text.paletteDisabled : item.note;
}

function paletteClose(): void {
  palette.hidden = true;
  paletteInput.value = "";
}

function paletteRun(item: PaletteItem): void {
  if (!item.button) {
    paletteNote.textContent = item.note; // stays open: there is nothing to run
    return;
  }
  if (item.button.disabled) {
    paletteNote.textContent = text.paletteDisabled;
    return;
  }
  paletteClose();
  item.button.click();
}

async function paletteOpen(): Promise<void> {
  if (!palette.hidden) {
    paletteInput.focus();
    return;
  }
  palette.hidden = false;
  paletteIndex = 0;
  paletteInput.focus();
  try {
    paletteItems = await buildPalette();
  } catch (error) {
    paletteClose();
    showError(error);
    return;
  }
  paletteRender();
}

element("palette-open").addEventListener("click", () => void paletteOpen());
paletteInput.addEventListener("input", () => {
  paletteIndex = 0;
  paletteRender();
});
paletteInput.addEventListener("keydown", (event) => {
  const last = paletteShown.length - 1;
  if (event.key === "ArrowDown") paletteSelect(Math.min(paletteIndex + 1, last));
  else if (event.key === "ArrowUp") paletteSelect(Math.max(paletteIndex - 1, 0));
  else if (event.key === "Enter") {
    const item = paletteShown[paletteIndex];
    if (item) paletteRun(item);
  } else if (event.key === "Escape") paletteClose();
  else return;
  event.preventDefault();
});
globalThis.document.addEventListener("pointerdown", (event) => {
  const target = event.target as Node;
  if (!palette.hidden && !palette.contains(target) && !element("palette-open").contains(target)) paletteClose();
});
window.addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === "k") {
    event.preventDefault();
    if (palette.hidden) void paletteOpen();
    else paletteClose();
  }
});
