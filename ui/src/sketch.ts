// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Sketch mode: draws one sketch flat and exact, and turns clicks into sketch
// commands. Every change goes through the API (architecture rule 2); this
// file only decides which command a click means.

import type {
  Constraint,
  DocumentState,
  Entity,
  Freedom,
  FreedomStatus,
  Geometry,
  Sketch,
  SketchRecord,
  XY,
} from "./api";

export type Tool = "select" | "line" | "rectangle" | "circle" | "arc" | "point" | "dimension";

/** What the editor needs from the app. */
export interface SketchHost {
  /** Runs a command; resolves once the new state has been rendered. */
  execute(command: string, args: Record<string, unknown>): Promise<DocumentState>;
  /** Shows a failure; returns the constraint ids it names, if any. */
  fail(error: unknown): number[];
  /** Tells the app the tool or selection changed, to update buttons. */
  changed(): void;
}

/** A position to draw from: an existing point, or a new position. */
type Ref = { point: number; at: XY } | { point: null; at: XY };

type Key = `e${number}` | `c${number}`;

const SVG = "http://www.w3.org/2000/svg";
const SNAP_PX = 9;
const PICK_PX = 7;

function svg<K extends keyof SVGElementTagNameMap>(
  tag: K,
  attributes: Record<string, string | number> = {},
): SVGElementTagNameMap[K] {
  const node = document.createElementNS(SVG, tag);
  for (const [name, value] of Object.entries(attributes)) node.setAttribute(name, String(value));
  return node;
}

function dist(a: XY, b: XY): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

function format(value: number): string {
  return String(Number(value.toFixed(3)));
}

/** Distance from p to segment ab. */
function segmentDistance(p: XY, a: XY, b: XY): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const length2 = dx * dx + dy * dy;
  const t = length2 > 0 ? Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / length2)) : 0;
  return dist(p, { x: a.x + t * dx, y: a.y + t * dy });
}

/** Is angle `a` within the counter-clockwise sweep from `from` to `to`? */
function inSweep(a: number, from: number, to: number): boolean {
  const tau = Math.PI * 2;
  const sweep = (((to - from) % tau) + tau) % tau;
  const offset = (((a - from) % tau) + tau) % tau;
  return offset <= sweep;
}

/** The angle a dimension between two lines measures, as the core does. */
function lineAngle(a: [XY, XY], b: [XY, XY]): number {
  const d1 = { x: a[1].x - a[0].x, y: a[1].y - a[0].y };
  const d2 = { x: b[1].x - b[0].x, y: b[1].y - b[0].y };
  const denom = d1.x * d2.y - d1.y * d2.x;
  let rev = [false, false];
  if (Math.abs(denom) > 1e-12 * Math.hypot(d1.x, d1.y) * Math.hypot(d2.x, d2.y)) {
    const w = { x: b[0].x - a[0].x, y: b[0].y - a[0].y };
    rev = [(w.x * d2.y - w.y * d2.x) / denom > 0.5, (w.x * d1.y - w.y * d1.x) / denom > 0.5];
  }
  const r1 = rev[0] ? { x: -d1.x, y: -d1.y } : d1;
  const r2 = rev[1] ? { x: -d2.x, y: -d2.y } : d2;
  return Math.abs((Math.atan2(r1.x * r2.y - r1.y * r2.x, r1.x * r2.x + r1.y * r2.y) * 180) / Math.PI);
}

const GLYPH: Record<string, string> = {
  horizontal: "H",
  horizontal_points: "H",
  vertical: "V",
  vertical_points: "V",
  parallel: "∥",
  perpendicular: "⊥",
  collinear: "≡",
  equal: "=",
  tangent: "T",
  concentric: "◎",
  midpoint: "M",
  symmetric: "S",
  fix: "F",
  coincident: "•",
  point_on_line: "•",
  point_on_curve: "•",
};

export class SketchEditor {
  readonly element: SVGSVGElement;
  private readonly grid = svg("g");
  private readonly reference = svg("path", { class: "reference" });
  /** Triangles of the face the sketch is on, in sketch coordinates. */
  private referenceTriangles: XY[] = [];
  private readonly shapes = svg("g");
  private readonly marks = svg("g");
  private readonly preview = svg("g");
  private readonly input: HTMLInputElement;

  private id: number | null = null;
  private record: SketchRecord | null = null;
  private freedom: Freedom | null = null;
  private scale = 4; // pixels per millimetre
  private origin = { x: 0, y: 0 }; // screen position of sketch (0, 0)
  tool: Tool = "select";
  selection: Key[] = [];
  private clicks: Ref[] = [];
  private cursor: XY = { x: 0, y: 0 };
  private flashed = new Set<number>();
  private flashTimer: number | undefined;
  private drag: { point: number; from: XY; moved: boolean } | null = null;
  private pan: { x: number; y: number } | null = null;
  private busy = false;
  /** The last dimension label pressed, to spot a double click by hand: the
   * labels are redrawn on every press, so the browser's own dblclick never
   * sees the same element twice. */
  private lastMark: { key: Key; time: number } | null = null;

  constructor(
    private readonly host: HTMLElement,
    private readonly app: SketchHost,
  ) {
    this.element = svg("svg", { class: "sketch-surface", tabindex: 0 });
    this.element.append(this.grid, this.reference, this.shapes, this.marks, this.preview);
    this.element.toggleAttribute("hidden", true);
    host.append(this.element);

    this.input = document.createElement("input");
    this.input.type = "number";
    this.input.step = "any";
    this.input.className = "dimension-input";
    this.input.hidden = true;
    host.append(this.input);

    this.element.addEventListener("pointerdown", (e) => this.down(e));
    this.element.addEventListener("pointermove", (e) => this.move(e));
    this.element.addEventListener("pointerup", (e) => this.up(e));
    this.element.addEventListener("contextmenu", (e) => e.preventDefault());
    this.element.addEventListener("wheel", (e) => this.wheel(e), { passive: false });
    new ResizeObserver(() => this.draw()).observe(host);
  }

  /** The sketch being edited, if any. */
  get editing(): number | null {
    return this.id;
  }

  open(id: number, state: DocumentState): void {
    this.id = id;
    this.selection = [];
    this.clicks = [];
    this.tool = "select";
    this.element.toggleAttribute("hidden", false);
    this.update(state);
    this.fit();
    this.element.focus();
  }

  close(): void {
    this.id = null;
    this.referenceTriangles = [];
    this.record = null;
    this.element.toggleAttribute("hidden", true);
    this.hideInput();
  }

  /** Re-reads the sketch after any change, including undo. */
  update(state: DocumentState): void {
    if (this.id === null) return;
    const record = state.document.sketches?.[String(this.id)];
    if (!record) {
      this.close();
      this.app.changed();
      return;
    }
    this.record = record;
    this.freedom = state.freedom[String(this.id)] ?? null;
    this.selection = this.selection.filter((key) => this.exists(key));
    this.draw();
  }

  /** Shows the face the sketch is on, as triangles in sketch coordinates
   * (three points each), or nothing. */
  setReference(triangles: XY[]): void {
    this.referenceTriangles = triangles;
    this.draw();
  }

  get sketchFreedom(): Freedom | null {
    return this.freedom;
  }

  setTool(tool: Tool): void {
    this.tool = tool;
    this.clicks = [];
    if (tool !== "select" && tool !== "dimension") this.selection = [];
    this.hideInput();
    this.draw();
    this.app.changed();
  }

  /** Highlights the constraints a conflict named. */
  flash(ids: number[]): void {
    this.flashed = new Set(ids);
    window.clearTimeout(this.flashTimer);
    this.flashTimer = window.setTimeout(() => {
      this.flashed.clear();
      this.draw();
    }, 4000);
    this.draw();
  }

  fit(): void {
    const { clientWidth: width, clientHeight: height } = this.host;
    const points = this.points();
    let box = { minX: -50, minY: -50, maxX: 50, maxY: 50 };
    if (points.length > 0 || this.referenceTriangles.length > 0) {
      box = { minX: Infinity, minY: Infinity, maxX: -Infinity, maxY: -Infinity };
      for (const p of this.referenceTriangles) {
        box.minX = Math.min(box.minX, p.x);
        box.maxX = Math.max(box.maxX, p.x);
        box.minY = Math.min(box.minY, p.y);
        box.maxY = Math.max(box.maxY, p.y);
      }
      for (const p of points) {
        const r = this.extent(p.id);
        box.minX = Math.min(box.minX, p.at.x - r);
        box.maxX = Math.max(box.maxX, p.at.x + r);
        box.minY = Math.min(box.minY, p.at.y - r);
        box.maxY = Math.max(box.maxY, p.at.y + r);
      }
    }
    const w = Math.max(box.maxX - box.minX, 10);
    const h = Math.max(box.maxY - box.minY, 10);
    this.scale = Math.min((width * 0.8) / w, (height * 0.8) / h);
    this.origin = {
      x: width / 2 - ((box.minX + box.maxX) / 2) * this.scale,
      y: height / 2 + ((box.minY + box.maxY) / 2) * this.scale,
    };
    this.draw();
  }

  // ---- commands from the ribbon ----

  /** Adds the constraint the selection implies for `kind`. */
  async constrain(kind: string): Promise<void> {
    const built = this.constraintsFor(kind);
    if (typeof built === "string") {
      this.app.fail(new Error(built));
      return;
    }
    for (const constraint of built) {
      if (!(await this.run("sketch.constrain", { constraint }))) return;
    }
    this.selection = [];
    this.draw();
    this.app.changed();
  }

  /** Toggles construction on the selected entities. */
  async toggleConstruction(): Promise<void> {
    const entities = this.selectedEntities();
    if (entities.length === 0) {
      this.app.fail(new Error("Select the lines, circles or arcs to make construction geometry."));
      return;
    }
    const make = !entities.every((id) => this.entity(id)?.construction === true);
    for (const entity of entities) {
      await this.run("sketch.set_construction", { entity, construction: make });
    }
  }

  /** Deletes the selection. */
  async deleteSelection(): Promise<void> {
    const keys = [...this.selection];
    this.selection = [];
    // Constraints first: deleting entities may take constraints with them.
    for (const key of keys.filter((k) => k.startsWith("c"))) {
      if (this.exists(key)) await this.run("sketch.remove_constraint", { constraint: Number(key.slice(1)) });
    }
    for (const key of keys.filter((k) => k.startsWith("e"))) {
      if (this.exists(key)) await this.run("sketch.remove_entity", { entity: Number(key.slice(1)) });
    }
    this.app.changed();
  }

  /** Escape: cancel what is in progress, then drop the selection, then the tool. */
  escape(): void {
    if (!this.input.hidden) this.hideInput();
    else if (this.clicks.length > 0) this.clicks = [];
    else if (this.selection.length > 0) this.selection = [];
    else this.tool = "select";
    this.draw();
    this.app.changed();
  }

  // ---- geometry lookups ----

  private get sketch(): Sketch | null {
    return this.record?.sketch ?? null;
  }

  private entity(id: number): Entity | undefined {
    return this.sketch?.entities[String(id)];
  }

  private geometry(id: number): Geometry | undefined {
    return this.entity(id)?.geometry;
  }

  private exists(key: Key): boolean {
    const id = key.slice(1);
    return key.startsWith("e")
      ? this.sketch?.entities[id] !== undefined
      : this.sketch?.constraints[id] !== undefined;
  }

  private at(id: number): XY | null {
    if (this.drag?.moved && this.drag.point === id) return this.cursor;
    const g = this.geometry(id);
    return g?.type === "point" ? g.at : null;
  }

  private points(): { id: number; at: XY }[] {
    const out: { id: number; at: XY }[] = [];
    for (const [key, entity] of Object.entries(this.sketch?.entities ?? {})) {
      if (entity.geometry.type === "point") {
        const at = this.at(Number(key));
        if (at) out.push({ id: Number(key), at });
      }
    }
    return out;
  }

  /** How far a point's drawing reaches: the radius of a circle centred on it. */
  private extent(point: number): number {
    let r = 0;
    for (const entity of Object.values(this.sketch?.entities ?? {})) {
      const g = entity.geometry;
      if (g.type === "circle" && g.center === point) r = Math.max(r, g.radius);
    }
    return r;
  }

  private line(id: number): [XY, XY] | null {
    const g = this.geometry(id);
    if (g?.type !== "line") return null;
    const a = this.at(g.start);
    const b = this.at(g.end);
    return a && b ? [a, b] : null;
  }

  private circle(id: number): { c: XY; r: number; from?: number; to?: number } | null {
    const g = this.geometry(id);
    if (g?.type === "circle") {
      const c = this.at(g.center);
      return c ? { c, r: g.radius } : null;
    }
    if (g?.type === "arc") {
      const c = this.at(g.center);
      const s = this.at(g.start);
      const e = this.at(g.end);
      if (!c || !s || !e) return null;
      return {
        c,
        r: dist(c, s),
        from: Math.atan2(s.y - c.y, s.x - c.x),
        to: Math.atan2(e.y - c.y, e.x - c.x),
      };
    }
    return null;
  }

  private kind(id: number): Geometry["type"] | undefined {
    return this.geometry(id)?.type;
  }

  private entityStatus(id: number): FreedomStatus {
    return this.freedom?.entities.find((e) => e.id === id)?.status ?? "under";
  }

  // ---- screen mapping ----

  private toScreen(p: XY): XY {
    return { x: this.origin.x + p.x * this.scale, y: this.origin.y - p.y * this.scale };
  }

  private toSketch(event: { clientX: number; clientY: number }): XY {
    const rect = this.element.getBoundingClientRect();
    return {
      x: (event.clientX - rect.left - this.origin.x) / this.scale,
      y: (this.origin.y - (event.clientY - rect.top)) / this.scale,
    };
  }

  // ---- picking ----

  /** The point under the cursor, for snapping. */
  private pointNear(at: XY, except?: number): number | null {
    let best: number | null = null;
    let bestDistance = SNAP_PX / this.scale;
    for (const p of this.points()) {
      if (p.id === except) continue;
      const d = dist(p.at, at);
      if (d < bestDistance) {
        best = p.id;
        bestDistance = d;
      }
    }
    return best;
  }

  /** The entity under the cursor: points first, then curves and lines. */
  private pick(at: XY): number | null {
    const point = this.pointNear(at);
    if (point !== null) return point;
    const limit = PICK_PX / this.scale;
    let best: number | null = null;
    let bestDistance = limit;
    for (const key of Object.keys(this.sketch?.entities ?? {})) {
      const id = Number(key);
      let d = Infinity;
      const l = this.line(id);
      if (l) d = segmentDistance(at, l[0], l[1]);
      const c = this.circle(id);
      if (c) {
        const angle = Math.atan2(at.y - c.c.y, at.x - c.c.x);
        const onArc = c.from === undefined || c.to === undefined || inSweep(angle, c.from, c.to);
        if (onArc) d = Math.abs(dist(at, c.c) - c.r);
      }
      if (d < bestDistance) {
        best = id;
        bestDistance = d;
      }
    }
    return best;
  }

  private ref(at: XY): Ref {
    const point = this.pointNear(at);
    if (point !== null) {
      const p = this.at(point);
      if (p) return { point, at: p };
    }
    // Snap to the sketch origin, so outlines can start exactly there.
    if (dist(at, { x: 0, y: 0 }) < SNAP_PX / this.scale) return { point: null, at: { x: 0, y: 0 } };
    return { point: null, at };
  }

  private static arg(ref: Ref): number | XY {
    return ref.point ?? ref.at;
  }

  // ---- events ----

  private down(event: PointerEvent): void {
    this.element.focus();
    if (event.button === 1 || event.button === 2) {
      this.pan = { x: event.clientX, y: event.clientY };
      this.element.setPointerCapture(event.pointerId);
      return;
    }
    if (event.button !== 0 || this.busy || this.id === null) return;
    const at = this.toSketch(event);
    this.cursor = at;
    const mark = this.pickMark(event);
    if (mark && this.lastMark?.key === mark && event.timeStamp - this.lastMark.time < 450) {
      this.lastMark = null;
      this.editDimension(mark, event);
      return;
    }
    this.lastMark = mark ? { key: mark, time: event.timeStamp } : null;
    if (this.tool === "select") {
      const hit = this.pickMark(event) ?? this.pick(at);
      const key = hit === null ? null : typeof hit === "string" ? hit : (`e${hit}` as Key);
      if (key === null) {
        if (!event.shiftKey) this.selection = [];
      } else if (event.shiftKey || event.metaKey || event.ctrlKey) {
        this.selection = this.selection.includes(key)
          ? this.selection.filter((k) => k !== key)
          : [...this.selection, key];
      } else if (!this.selection.includes(key)) {
        this.selection = [key];
      }
      if (typeof hit === "number" && this.kind(hit) === "point") {
        this.drag = { point: hit, from: at, moved: false };
        this.element.setPointerCapture(event.pointerId);
      }
      this.draw();
      this.app.changed();
      return;
    }
    if (this.tool === "dimension") {
      const hit = this.pick(at);
      if (hit === null) return;
      const key = `e${hit}` as Key;
      if (!this.selection.includes(key)) this.selection = [...this.selection, key];
      this.offerDimension();
      this.draw();
      return;
    }
    this.clicks.push(this.ref(at));
    void this.complete();
  }

  private move(event: PointerEvent): void {
    if (this.pan) {
      this.origin.x += event.clientX - this.pan.x;
      this.origin.y += event.clientY - this.pan.y;
      this.pan = { x: event.clientX, y: event.clientY };
      this.draw();
      return;
    }
    this.cursor = this.toSketch(event);
    if (this.drag && dist(this.cursor, this.drag.from) * this.scale > 3) this.drag.moved = true;
    this.draw();
  }

  private up(event: PointerEvent): void {
    if (this.pan) {
      this.pan = null;
      return;
    }
    const drag = this.drag;
    this.drag = null;
    if (drag?.moved) {
      const to = this.toSketch(event);
      void this.run("sketch.move_point", { point: drag.point, to });
    }
  }

  /** Opens the value of a dimension for editing. */
  private editDimension(mark: Key, where: { clientX: number; clientY: number }): void {
    const id = Number(mark.slice(1));
    const constraint = this.sketch?.constraints[String(id)];
    if (constraint?.value === undefined) return;
    this.askValue(where, constraint.value, async (value) => {
      await this.run("sketch.set_dimension", { constraint: id, value });
    });
  }

  private wheel(event: WheelEvent): void {
    event.preventDefault();
    const rect = this.element.getBoundingClientRect();
    const sx = event.clientX - rect.left;
    const sy = event.clientY - rect.top;
    const factor = Math.exp(-event.deltaY * 0.0015);
    const scale = Math.min(Math.max(this.scale * factor, 0.01), 10_000);
    this.origin.x = sx - ((sx - this.origin.x) * scale) / this.scale;
    this.origin.y = sy - ((sy - this.origin.y) * scale) / this.scale;
    this.scale = scale;
    this.draw();
  }

  /** A dimension label or constraint glyph under the pointer. */
  private pickMark(event: { target: EventTarget | null }): Key | null {
    const target = event.target as Element | null;
    const key = target?.closest("[data-key]")?.getAttribute("data-key");
    return key && /^c\d+$/.test(key) ? (key as Key) : null;
  }

  // ---- drawing tools ----

  private async complete(): Promise<void> {
    const [a, b, c] = this.clicks;
    if (!a) return;
    switch (this.tool) {
      case "point":
        this.clicks = [];
        await this.run("sketch.add_point", { at: a.at });
        break;
      case "line": {
        if (!b) return;
        if (dist(a.at, b.at) * this.scale < 2 || (a.point !== null && a.point === b.point)) {
          // A click on the last point ends the chain.
          this.clicks = [];
          break;
        }
        const before = this.nextEntity();
        const state = await this.run("sketch.add_line", {
          start: SketchEditor.arg(a),
          end: SketchEditor.arg(b),
        });
        // Keep drawing from the end, like a polyline.
        const end = b.point ?? before + (a.point === null ? 1 : 0);
        this.clicks = state && this.at(end) ? [{ point: end, at: b.at }] : [];
        break;
      }
      case "rectangle":
        if (!b) return;
        this.clicks = [];
        await this.run("sketch.add_rectangle", { from: a.at, to: b.at });
        break;
      case "circle":
        if (!b) return;
        this.clicks = [];
        await this.run("sketch.add_circle", { center: SketchEditor.arg(a), radius: dist(a.at, b.at) });
        break;
      case "arc":
        if (!b || !c) return;
        this.clicks = [];
        await this.run("sketch.add_arc", {
          center: SketchEditor.arg(a),
          start: SketchEditor.arg(b),
          end: SketchEditor.arg(c),
        });
        break;
      default:
        this.clicks = [];
    }
    this.draw();
  }

  private nextEntity(): number {
    return this.sketch?.next_entity ?? 0;
  }

  /** Runs a sketch command; on failure flashes what it names. */
  private async run(command: string, args: Record<string, unknown>): Promise<DocumentState | null> {
    if (this.id === null) return null;
    this.busy = true;
    try {
      return await this.app.execute(command, { sketch: this.id, ...args });
    } catch (error) {
      const ids = this.app.fail(error);
      if (ids.length > 0) this.flash(ids);
      return null;
    } finally {
      this.busy = false;
    }
  }

  // ---- constraints and dimensions ----

  private selectedEntities(): number[] {
    return this.selection.filter((k) => k.startsWith("e")).map((k) => Number(k.slice(1)));
  }

  /** Builds the constraints `kind` means for the current selection, or explains what to pick. */
  constraintsFor(kind: string): Constraint[] | string {
    const ids = this.selectedEntities();
    const points = ids.filter((id) => this.kind(id) === "point");
    const lines = ids.filter((id) => this.kind(id) === "line");
    const curves = ids.filter((id) => this.kind(id) === "circle" || this.kind(id) === "arc");
    const [p0, p1] = points;
    const [l0, l1] = lines;
    const [c0, c1] = curves;
    const only = (n: number) => ids.length === n;
    switch (kind) {
      case "coincident":
        if (only(2) && p0 !== undefined && p1 !== undefined) return [{ type: "coincident", a: p0, b: p1 }];
        if (only(2) && p0 !== undefined && l0 !== undefined) return [{ type: "point_on_line", point: p0, line: l0 }];
        if (only(2) && p0 !== undefined && c0 !== undefined) return [{ type: "point_on_curve", point: p0, curve: c0 }];
        return "Coincident: select two points, or a point and a line, circle or arc.";
      case "horizontal":
      case "vertical":
        if (lines.length > 0 && lines.length === ids.length) return lines.map((line) => ({ type: kind, line }));
        if (only(2) && p0 !== undefined && p1 !== undefined) return [{ type: `${kind}_points`, a: p0, b: p1 }];
        return `${kind === "horizontal" ? "Horizontal" : "Vertical"}: select lines, or two points.`;
      case "parallel":
      case "perpendicular":
      case "collinear":
        if (only(2) && l0 !== undefined && l1 !== undefined) return [{ type: kind, a: l0, b: l1 }];
        return `${kind[0]?.toUpperCase()}${kind.slice(1)}: select two lines.`;
      case "equal":
        if (only(2) && l0 !== undefined && l1 !== undefined) return [{ type: "equal", a: l0, b: l1 }];
        if (only(2) && c0 !== undefined && c1 !== undefined) return [{ type: "equal", a: c0, b: c1 }];
        return "Equal: select two lines, or two circles or arcs.";
      case "tangent":
        if (only(2) && l0 !== undefined && c0 !== undefined) return [{ type: "tangent", a: l0, b: c0 }];
        if (only(2) && c0 !== undefined && c1 !== undefined) return [{ type: "tangent", a: c0, b: c1 }];
        return "Tangent: select a line and a circle or arc, or two circles or arcs.";
      case "concentric":
        if (only(2) && c0 !== undefined && c1 !== undefined) return [{ type: "concentric", a: c0, b: c1 }];
        return "Concentric: select two circles or arcs.";
      case "midpoint":
        if (only(2) && p0 !== undefined && l0 !== undefined) return [{ type: "midpoint", point: p0, line: l0 }];
        return "Midpoint: select a point and a line.";
      case "symmetric":
        if (only(3) && p0 !== undefined && p1 !== undefined && l0 !== undefined)
          return [{ type: "symmetric", a: p0, b: p1, axis: l0 }];
        return "Symmetric: select two points and the line to mirror them about.";
      case "fix":
        if (points.length > 0 && points.length === ids.length) return points.map((point) => ({ type: "fix", point }));
        return "Fix: select points.";
      default:
        return `Unknown constraint ${kind}.`;
    }
  }

  /** The dimension the selection makes, with its current value. */
  private dimension(): { constraint: Constraint; value: number } | null {
    const ids = this.selectedEntities();
    const [a, b] = ids;
    if (a === undefined) return null;
    const ka = this.kind(a);
    if (ids.length === 1) {
      const l = this.line(a);
      if (l) return { constraint: { type: "length", line: a }, value: dist(l[0], l[1]) };
      const c = this.circle(a);
      if (c && ka === "circle") return { constraint: { type: "diameter", curve: a }, value: 2 * c.r };
      if (c) return { constraint: { type: "radius", curve: a }, value: c.r };
      return null;
    }
    if (ids.length !== 2 || b === undefined) return null;
    const kb = this.kind(b);
    if (ka === "point" && kb === "point") {
      const pa = this.at(a);
      const pb = this.at(b);
      if (!pa || !pb) return null;
      return { constraint: { type: "distance", a, b, axis: "aligned" }, value: dist(pa, pb) };
    }
    if ((ka === "point" && kb === "line") || (ka === "line" && kb === "point")) {
      const point = ka === "point" ? a : b;
      const line = ka === "line" ? a : b;
      const p = this.at(point);
      const l = this.line(line);
      if (!p || !l) return null;
      const length = dist(l[0], l[1]);
      const cross = ((l[1].x - l[0].x) * (p.y - l[0].y) - (l[1].y - l[0].y) * (p.x - l[0].x)) / length;
      return { constraint: { type: "point_line_distance", point, line }, value: Math.abs(cross) };
    }
    if (ka === "line" && kb === "line") {
      const la = this.line(a);
      const lb = this.line(b);
      if (!la || !lb) return null;
      return { constraint: { type: "angle", a, b }, value: lineAngle(la, lb) };
    }
    return null;
  }

  private offerDimension(): void {
    const found = this.dimension();
    if (!found) {
      this.hideInput();
      if (this.selection.length >= 2) {
        this.app.fail(new Error("Dimension: pick a line, circle, arc, two points, a point and a line, or two lines."));
        this.selection = [];
      }
      return;
    }
    const at = this.toScreen(this.cursor);
    const rect = this.element.getBoundingClientRect();
    this.askValue(
      { clientX: rect.left + at.x, clientY: rect.top + at.y },
      found.value,
      async (value) => {
        await this.run("sketch.constrain", { constraint: { ...found.constraint, value } });
        this.selection = [];
        this.draw();
      },
    );
  }

  private askValue(
    where: { clientX: number; clientY: number },
    value: number,
    commit: (value: number) => Promise<void>,
  ): void {
    const rect = this.host.getBoundingClientRect();
    this.input.style.left = `${where.clientX - rect.left + 12}px`;
    this.input.style.top = `${where.clientY - rect.top - 14}px`;
    this.input.value = format(value);
    this.input.hidden = false;
    this.input.focus();
    this.input.select();
    this.input.onkeydown = (event) => {
      event.stopPropagation();
      if (event.key === "Escape") {
        this.hideInput();
        this.selection = [];
        this.draw();
        this.element.focus();
      } else if (event.key === "Enter") {
        const next = Number(this.input.value);
        this.hideInput();
        this.element.focus();
        if (Number.isFinite(next)) void commit(next);
      }
    };
  }

  private hideInput(): void {
    this.input.hidden = true;
    this.input.onkeydown = null;
  }

  // ---- rendering ----

  draw(): void {
    if (this.id === null || this.element.hasAttribute("hidden")) return;
    const { clientWidth: width, clientHeight: height } = this.host;
    this.element.setAttribute("viewBox", `0 0 ${width} ${height}`);
    this.drawGrid(width, height);
    let d = "";
    for (let i = 0; i + 2 < this.referenceTriangles.length; i += 3) {
      const [a, b, c] = [0, 1, 2].map((k) => this.toScreen(this.referenceTriangles[i + k] ?? { x: 0, y: 0 }));
      if (a && b && c) d += `M${a.x} ${a.y}L${b.x} ${b.y}L${c.x} ${c.y}Z`;
    }
    this.reference.setAttribute("d", d);
    this.drawShapes();
    this.drawMarks();
    this.drawPreview();
  }

  private drawGrid(width: number, height: number): void {
    this.grid.replaceChildren();
    // A step that is 1, 2 or 5 times a power of ten, at least 14 px apart.
    const raw = 14 / this.scale;
    const power = 10 ** Math.floor(Math.log10(raw));
    const step = [1, 2, 5, 10].map((m) => m * power).find((s) => s >= raw) ?? power * 10;
    const left = -this.origin.x / this.scale;
    const right = (width - this.origin.x) / this.scale;
    const top = this.origin.y / this.scale;
    const bottom = (this.origin.y - height) / this.scale;
    let path = "";
    let major = "";
    for (let x = Math.ceil(left / step) * step; x <= right; x += step) {
      const sx = this.origin.x + x * this.scale;
      const line = `M${sx} 0V${height}`;
      if (Math.round(x / step) % 5 === 0) major += line;
      else path += line;
    }
    for (let y = Math.ceil(bottom / step) * step; y <= top; y += step) {
      const sy = this.origin.y - y * this.scale;
      const line = `M0 ${sy}H${width}`;
      if (Math.round(y / step) % 5 === 0) major += line;
      else path += line;
    }
    this.grid.append(
      svg("path", { d: path, class: "grid-minor" }),
      svg("path", { d: major, class: "grid-major" }),
      svg("path", { d: `M0 ${this.origin.y}H${width}`, class: "axis-x" }),
      svg("path", { d: `M${this.origin.x} 0V${height}`, class: "axis-y" }),
    );
  }

  private classes(id: number, base: string): string {
    const parts = [base, `status-${this.entityStatus(id)}`];
    if (this.entity(id)?.construction) parts.push("construction");
    if (this.selection.includes(`e${id}`)) parts.push("selected");
    return parts.join(" ");
  }

  private drawShapes(): void {
    this.shapes.replaceChildren();
    const pointNodes: SVGElement[] = [];
    for (const [key, entity] of Object.entries(this.sketch?.entities ?? {})) {
      const id = Number(key);
      const g = entity.geometry;
      if (g.type === "line") {
        const l = this.line(id);
        if (!l) continue;
        const [a, b] = [this.toScreen(l[0]), this.toScreen(l[1])];
        this.shapes.append(
          svg("line", { x1: a.x, y1: a.y, x2: b.x, y2: b.y, class: this.classes(id, "curve"), "data-key": `e${id}` }),
        );
      } else if (g.type === "circle" || g.type === "arc") {
        const c = this.circle(id);
        if (!c) continue;
        const center = this.toScreen(c.c);
        const r = c.r * this.scale;
        if (c.from === undefined || c.to === undefined) {
          this.shapes.append(
            svg("circle", { cx: center.x, cy: center.y, r, class: this.classes(id, "curve"), "data-key": `e${id}` }),
          );
        } else {
          const tau = Math.PI * 2;
          const sweep = (((c.to - c.from) % tau) + tau) % tau;
          const s = { x: center.x + r * Math.cos(c.from), y: center.y - r * Math.sin(c.from) };
          const e = { x: center.x + r * Math.cos(c.to), y: center.y - r * Math.sin(c.to) };
          // Screen Y points down, so a counter-clockwise sketch arc is clockwise on screen.
          const d = `M${s.x} ${s.y}A${r} ${r} 0 ${sweep > Math.PI ? 1 : 0} 0 ${e.x} ${e.y}`;
          this.shapes.append(svg("path", { d, class: this.classes(id, "curve"), "data-key": `e${id}` }));
        }
      } else {
        const at = this.at(id);
        if (!at) continue;
        const s = this.toScreen(at);
        pointNodes.push(svg("circle", { cx: s.x, cy: s.y, r: 3.2, class: this.classes(id, "point"), "data-key": `e${id}` }));
      }
    }
    // Points on top, so they stay easy to pick.
    this.shapes.append(...pointNodes);
  }

  /** Where a constraint's glyph or label goes, in screen pixels. */
  private anchor(constraint: Constraint): XY | null {
    const refs = ["line", "curve", "point", "a"].map((k) => constraint[k]).filter((v): v is number => typeof v === "number");
    const first = refs[0];
    if (first === undefined) return null;
    const l = this.line(first);
    if (l) {
      const mid = this.toScreen({ x: (l[0].x + l[1].x) / 2, y: (l[0].y + l[1].y) / 2 });
      const dx = l[1].x - l[0].x;
      const dy = l[1].y - l[0].y;
      const n = Math.hypot(dx, dy) || 1;
      return { x: mid.x - (dy / n) * 14, y: mid.y - (dx / n) * 14 };
    }
    const c = this.circle(first);
    if (c) {
      const angle = c.from !== undefined && c.to !== undefined ? c.from + 0.4 : Math.PI / 4;
      return this.toScreen({ x: c.c.x + c.r * Math.cos(angle), y: c.c.y + c.r * Math.sin(angle) });
    }
    const p = this.at(first);
    return p ? this.toScreen(p) : null;
  }

  private drawMarks(): void {
    this.marks.replaceChildren();
    const stacked = new Map<string, number>();
    for (const [key, constraint] of Object.entries(this.sketch?.constraints ?? {})) {
      const id = Number(key);
      const at = this.anchor(constraint);
      if (!at) continue;
      const slot = `${Math.round(at.x)},${Math.round(at.y)}`;
      const n = stacked.get(slot) ?? 0;
      stacked.set(slot, n + 1);
      const classes = ["mark"];
      if (this.selection.includes(`c${id}`)) classes.push("selected");
      if (this.flashed.has(id)) classes.push("flashed");
      const group = svg("g", { class: classes.join(" "), "data-key": `c${id}` });
      const label =
        constraint.value !== undefined ? this.label(constraint) : (GLYPH[constraint.type] ?? "?");
      const width = Math.max(16, label.length * 7 + 8);
      const x = at.x + n * (width + 3) + 6;
      const y = at.y + 6;
      group.append(
        svg("rect", { x, y: y - 10, width, height: 16, rx: 5, class: constraint.value !== undefined ? "dim-box" : "glyph-box" }),
      );
      const text = svg("text", { x: x + width / 2, y: y + 2, "text-anchor": "middle" });
      text.textContent = label;
      group.append(text);
      const title = svg("title");
      title.textContent = `${constraint.type.replaceAll("_", " ")}${constraint.value !== undefined ? ` (double-click to change)` : ""}`;
      group.append(title);
      this.marks.append(group);
    }
  }

  private label(constraint: Constraint): string {
    const value = format(constraint.value ?? 0);
    switch (constraint.type) {
      case "angle":
        return `${value}°`;
      case "radius":
        return `R${value}`;
      case "diameter":
        return `⌀${value}`;
      default:
        return value;
    }
  }

  private drawPreview(): void {
    this.preview.replaceChildren();
    if (this.tool === "select" || this.tool === "dimension") return;
    const snap = this.ref(this.cursor);
    const cursor = this.toScreen(snap.at);
    if (snap.point !== null || (snap.at.x === 0 && snap.at.y === 0)) {
      this.preview.append(svg("circle", { cx: cursor.x, cy: cursor.y, r: 7, class: "snap" }));
    }
    const [a, b] = this.clicks;
    if (!a) return;
    const pa = this.toScreen(a.at);
    switch (this.tool) {
      case "line":
        this.preview.append(svg("line", { x1: pa.x, y1: pa.y, x2: cursor.x, y2: cursor.y, class: "ghost" }));
        break;
      case "rectangle":
        this.preview.append(
          svg("rect", {
            x: Math.min(pa.x, cursor.x),
            y: Math.min(pa.y, cursor.y),
            width: Math.abs(cursor.x - pa.x),
            height: Math.abs(cursor.y - pa.y),
            class: "ghost",
          }),
        );
        break;
      case "circle":
        this.preview.append(svg("circle", { cx: pa.x, cy: pa.y, r: dist(pa, cursor), class: "ghost" }));
        break;
      case "arc": {
        if (!b) {
          this.preview.append(svg("line", { x1: pa.x, y1: pa.y, x2: cursor.x, y2: cursor.y, class: "ghost" }));
          break;
        }
        const pb = this.toScreen(b.at);
        const r = dist(pa, pb);
        const angle = Math.atan2(cursor.y - pa.y, cursor.x - pa.x);
        const end = { x: pa.x + r * Math.cos(angle), y: pa.y + r * Math.sin(angle) };
        const from = Math.atan2(-(pb.y - pa.y), pb.x - pa.x);
        const to = Math.atan2(-(end.y - pa.y), end.x - pa.x);
        const tau = Math.PI * 2;
        const sweep = (((to - from) % tau) + tau) % tau;
        this.preview.append(
          svg("path", { d: `M${pb.x} ${pb.y}A${r} ${r} 0 ${sweep > Math.PI ? 1 : 0} 0 ${end.x} ${end.y}`, class: "ghost" }),
        );
        break;
      }
      default:
        break;
    }
  }
}
