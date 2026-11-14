// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The 3D viewport. Z is up, as on a machine tool. Units are millimetres.

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { Frame, Mesh, SketchRecord, XY } from "./api";

/** A sketch point in model coordinates. */
function onFrame(frame: Frame, p: XY): THREE.Vector3 {
  const { origin: o, x, y } = frame;
  return new THREE.Vector3(
    o.x + p.x * x[0] + p.y * y[0],
    o.y + p.x * x[1] + p.y * y[1],
    o.z + p.x * x[2] + p.y * y[2],
  );
}

/** Line segments (pairs of sketch points) tracing a sketch's curves. */
function sketchSegments(record: SketchRecord): { segments: XY[]; construction: XY[] } {
  const segments: XY[] = [];
  const construction: XY[] = [];
  const { entities } = record.sketch;
  const at = (id: number): XY | null => {
    const g = entities[String(id)]?.geometry;
    return g?.type === "point" ? g.at : null;
  };
  const ring = (out: XY[], c: XY, r: number, from: number, sweep: number) => {
    const steps = Math.max(8, Math.ceil((sweep / (Math.PI * 2)) * 72));
    for (let i = 0; i < steps; i++) {
      const a0 = from + (sweep * i) / steps;
      const a1 = from + (sweep * (i + 1)) / steps;
      out.push(
        { x: c.x + r * Math.cos(a0), y: c.y + r * Math.sin(a0) },
        { x: c.x + r * Math.cos(a1), y: c.y + r * Math.sin(a1) },
      );
    }
  };
  for (const entity of Object.values(entities)) {
    const out = entity.construction ? construction : segments;
    const g = entity.geometry;
    if (g.type === "line") {
      const a = at(g.start);
      const b = at(g.end);
      if (a && b) out.push(a, b);
    } else if (g.type === "circle") {
      const c = at(g.center);
      if (c) ring(out, c, g.radius, 0, Math.PI * 2);
    } else if (g.type === "arc") {
      const c = at(g.center);
      const s = at(g.start);
      const e = at(g.end);
      if (!c || !s || !e) continue;
      const from = Math.atan2(s.y - c.y, s.x - c.x);
      const to = Math.atan2(e.y - c.y, e.x - c.x);
      const sweep = (((to - from) % (Math.PI * 2)) + Math.PI * 2) % (Math.PI * 2);
      ring(out, c, Math.hypot(s.x - c.x, s.y - c.y), from, sweep);
    }
  }
  return { segments, construction };
}

/** The standard views: where the camera looks from, and which way is up. */
export const VIEWS = {
  home: { from: [1, -1.4, 1], up: [0, 0, 1] },
  // A hair off vertical: orbiting keeps Z up, which a camera straight
  // above cannot have.
  top: { from: [0, -1e-4, 1], up: [0, 0, 1] },
  bottom: { from: [0, -1e-4, -1], up: [0, 0, 1] },
  front: { from: [0, -1, 0], up: [0, 0, 1] },
  back: { from: [0, 1, 0], up: [0, 0, 1] },
  right: { from: [1, 0, 0], up: [0, 0, 1] },
  left: { from: [-1, 0, 0], up: [0, 0, 1] },
} as const;
export type ViewName = keyof typeof VIEWS;

/** What can be shown or hidden, like layers. */
export type Layer = "bodies" | "sketches" | "toolpaths" | "grid";

/** A face of the view cube: its label, and the view it turns to. */
const CUBE_FACES: { name: ViewName; label: string }[] = [
  // BoxGeometry's face order: +x, −x, +y, −y, +z, −z.
  { name: "right", label: "RIGHT" },
  { name: "left", label: "LEFT" },
  { name: "back", label: "BACK" },
  { name: "front", label: "FRONT" },
  { name: "top", label: "TOP" },
  { name: "bottom", label: "BOTTOM" },
];

/**
 * A small cube in the corner that turns with the view. Clicking a face
 * looks at the part from that side.
 */
class ViewCube {
  private readonly renderer: THREE.WebGLRenderer;
  private readonly scene = new THREE.Scene();
  private readonly camera = new THREE.OrthographicCamera(-1.6, 1.6, 1.6, -1.6, 0.1, 10);
  private readonly cube: THREE.Mesh;
  private readonly raycaster = new THREE.Raycaster();

  constructor(host: HTMLElement, onPick: (view: ViewName) => void) {
    this.renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    this.renderer.setSize(104, 104, false);
    const canvas = this.renderer.domElement;
    canvas.className = "view-cube";
    canvas.title = "Click a face to look from that side";
    host.appendChild(canvas);
    const materials = CUBE_FACES.map(({ label }) => {
      const paint = globalThis.document.createElement("canvas");
      paint.width = paint.height = 128;
      const g = paint.getContext("2d");
      if (g) {
        g.fillStyle = "#2b3138";
        g.fillRect(0, 0, 128, 128);
        g.strokeStyle = "#5a6470";
        g.lineWidth = 6;
        g.strokeRect(3, 3, 122, 122);
        g.fillStyle = "#d7dde4";
        g.font = "600 26px system-ui, sans-serif";
        g.textAlign = "center";
        g.textBaseline = "middle";
        g.fillText(label, 64, 64);
      }
      const texture = new THREE.CanvasTexture(paint);
      texture.colorSpace = THREE.SRGBColorSpace;
      return new THREE.MeshBasicMaterial({ map: texture });
    });
    this.cube = new THREE.Mesh(new THREE.BoxGeometry(1.6, 1.6, 1.6), materials);
    this.scene.add(this.cube);
    this.camera.up.set(0, 0, 1);
    canvas.addEventListener("click", (event) => {
      const rect = canvas.getBoundingClientRect();
      const pointer = new THREE.Vector2(
        ((event.clientX - rect.left) / rect.width) * 2 - 1,
        -((event.clientY - rect.top) / rect.height) * 2 + 1,
      );
      this.raycaster.setFromCamera(pointer, this.camera);
      const hit = this.raycaster.intersectObject(this.cube)[0];
      const face = hit?.face ? CUBE_FACES[hit.face.materialIndex] : undefined;
      if (face) onPick(face.name);
    });
  }

  /** Turns the cube to match the main camera. */
  render(main: THREE.PerspectiveCamera, target: THREE.Vector3): void {
    const direction = main.position.clone().sub(target).normalize();
    this.camera.position.copy(direction.multiplyScalar(4));
    this.camera.up.copy(main.up);
    this.camera.lookAt(0, 0, 0);
    this.renderer.render(this.scene, this.camera);
  }
}

export class Viewer {
  private readonly renderer: THREE.WebGLRenderer;
  private readonly scene = new THREE.Scene();
  private readonly camera = new THREE.PerspectiveCamera(40, 1, 0.1, 100_000);
  private readonly controls: OrbitControls;
  private readonly grid: THREE.GridHelper;
  private model: THREE.Group | null = null;
  private surface: THREE.Mesh | null = null;
  private triangleFaces: Uint32Array | null = null;
  private highlight: THREE.Group | null = null;
  private edgeLines: Float32Array[] = [];
  private edgeHighlight: THREE.Group | null = null;
  private toolpath: THREE.Group | null = null;
  private stock: THREE.Mesh | null = null;
  private readonly cube: ViewCube;
  private readonly hidden = new Set<Layer>();
  private wireframe = false;
  private readonly raycaster = new THREE.Raycaster();
  private sketches = new THREE.Group();

  constructor(private readonly host: HTMLElement) {
    this.renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    host.appendChild(this.renderer.domElement);

    THREE.Object3D.DEFAULT_UP.set(0, 0, 1);
    this.camera.up.set(0, 0, 1);
    this.camera.position.set(160, -220, 160);

    this.scene.add(new THREE.HemisphereLight(0xdfe6ee, 0x30363d, 1.6));
    const key = new THREE.DirectionalLight(0xffffff, 1.6);
    key.position.set(1, -2, 3);
    this.scene.add(key);
    // A soft light from below, so undersides (a part's second setup) read.
    const fill = new THREE.DirectionalLight(0xffffff, 0.7);
    fill.position.set(-1, 1.5, -2.5);
    this.scene.add(fill);

    // GridHelper lies in XZ; turn it into the XY plane.
    this.grid = new THREE.GridHelper(200, 20, 0x3a4149, 0x262b31);
    this.grid.rotation.x = Math.PI / 2;
    this.scene.add(this.grid);
    this.scene.add(new THREE.AxesHelper(20));
    this.scene.add(this.sketches);

    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;
    this.controls.addEventListener("change", () => this.render());

    this.cube = new ViewCube(host, (view) => this.look(view));

    new ResizeObserver(() => this.resize()).observe(host);
    this.resize();
    this.loop();
  }

  /** Shows a mesh, replacing anything shown before, and frames it unless
   * `keepView` is set (for a rebuild while the user is looking). */
  show(mesh: Mesh, keepView = false): void {
    this.clear();
    this.setStock(null);
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(mesh.positions, 3));
    geometry.setIndex(new THREE.BufferAttribute(mesh.indices, 1));
    // Every face has its own vertices, so this is smooth within a face and
    // sharp across face edges.
    geometry.computeVertexNormals();
    geometry.computeBoundingSphere();

    const surface = new THREE.Mesh(
      geometry,
      new THREE.MeshStandardMaterial({
        color: 0xaeb6bd,
        metalness: 0.25,
        roughness: 0.55,
        polygonOffset: true,
        polygonOffsetFactor: 1,
        polygonOffsetUnits: 1,
      }),
    );
    const edges = new THREE.LineSegments(
      new THREE.EdgesGeometry(geometry, 25),
      new THREE.LineBasicMaterial({ color: 0x0e1013 }),
    );
    this.surface = surface;
    this.triangleFaces = mesh.triangleFaces;
    this.model = new THREE.Group();
    this.model.add(surface, edges);
    surface.visible = !this.wireframe;
    this.model.visible = !this.hidden.has("bodies");
    this.scene.add(this.model);
    if (keepView) this.render();
    else this.fit();
  }

  /** The face number under a point of the canvas, or null. */
  faceAt(clientX: number, clientY: number): number | null {
    if (!this.surface || !this.triangleFaces) return null;
    const rect = this.renderer.domElement.getBoundingClientRect();
    const pointer = new THREE.Vector2(
      ((clientX - rect.left) / rect.width) * 2 - 1,
      -((clientY - rect.top) / rect.height) * 2 + 1,
    );
    this.raycaster.setFromCamera(pointer, this.camera);
    const hit = this.raycaster.intersectObject(this.surface)[0];
    if (hit?.faceIndex === undefined || hit.faceIndex === null) return null;
    return this.triangleFaces[hit.faceIndex] ?? null;
  }

  /** Tints one face, or none. */
  highlightFace(face: number | null): void {
    this.highlightFaces(face === null ? [] : [face]);
  }

  /** Tints picked faces, and more strongly the one under the pointer. */
  highlightFaces(faces: number[], hover: number | null = null): void {
    if (this.highlight) {
      this.scene.remove(this.highlight);
      this.highlight.traverse((object) => {
        if (object instanceof THREE.Mesh) {
          object.geometry.dispose();
          (object.material as THREE.Material).dispose();
        }
      });
      this.highlight = null;
    }
    const source = this.surface?.geometry;
    const index = source?.getIndex();
    if (source && index && this.triangleFaces) {
      const group = new THREE.Group();
      const tint = (wanted: Set<number>, color: number, opacity: number) => {
        const picked: number[] = [];
        this.triangleFaces?.forEach((f, t) => {
          if (wanted.has(f)) picked.push(index.getX(t * 3), index.getX(t * 3 + 1), index.getX(t * 3 + 2));
        });
        if (picked.length === 0) return;
        const geometry = new THREE.BufferGeometry();
        geometry.setAttribute("position", source.getAttribute("position"));
        geometry.setIndex(picked);
        group.add(
          new THREE.Mesh(
            geometry,
            new THREE.MeshBasicMaterial({
              color,
              transparent: true,
              opacity,
              polygonOffset: true,
              polygonOffsetFactor: -1,
              polygonOffsetUnits: -1,
            }),
          ),
        );
      };
      tint(new Set(faces), 0xffb454, 0.55);
      if (hover !== null && !faces.includes(hover)) tint(new Set([hover]), 0xffb454, 0.3);
      this.highlight = group;
      this.scene.add(group);
    }
    this.render();
  }

  /** Keeps the part's edges, for picking. */
  setEdges(lines: Float32Array[]): void {
    this.edgeLines = lines;
    this.highlightEdges([]);
  }

  /** The edge near a point of the canvas (within 7 px, not hidden behind
   * the part), or null. */
  edgeAt(clientX: number, clientY: number): number | null {
    const rect = this.renderer.domElement.getBoundingClientRect();
    const px = clientX - rect.left;
    const py = clientY - rect.top;
    const toScreen = (v: THREE.Vector3) => {
      const p = v.clone().project(this.camera);
      return { x: ((p.x + 1) / 2) * rect.width, y: ((1 - p.y) / 2) * rect.height, z: p.z };
    };
    let best: { edge: number; distance: number; point: THREE.Vector3 } | null = null;
    this.edgeLines.forEach((line, edge) => {
      for (let k = 0; k + 5 < line.length; k += 3) {
        const a = new THREE.Vector3(line[k], line[k + 1], line[k + 2]);
        const b = new THREE.Vector3(line[k + 3], line[k + 4], line[k + 5]);
        const sa = toScreen(a);
        const sb = toScreen(b);
        if (sa.z > 1 || sb.z > 1) continue;
        const dx = sb.x - sa.x;
        const dy = sb.y - sa.y;
        const len2 = dx * dx + dy * dy;
        const t = len2 > 0 ? Math.max(0, Math.min(1, ((px - sa.x) * dx + (py - sa.y) * dy) / len2)) : 0;
        const d = Math.hypot(px - (sa.x + t * dx), py - (sa.y + t * dy));
        if (d < 7 && (!best || d < best.distance)) best = { edge, distance: d, point: a.clone().lerp(b, t) };
      }
    });
    if (!best) return null;
    const found: { edge: number; distance: number; point: THREE.Vector3 } = best;
    // Hidden behind the part? Compare with the surface under the pointer.
    if (this.surface) {
      const pointer = new THREE.Vector2((px / rect.width) * 2 - 1, -(py / rect.height) * 2 + 1);
      this.raycaster.setFromCamera(pointer, this.camera);
      const hit = this.raycaster.intersectObject(this.surface)[0];
      const edgeDistance = this.camera.position.distanceTo(found.point);
      if (hit && edgeDistance > hit.distance * 1.01 + 0.05) return null;
    }
    return found.edge;
  }

  /** Draws picked edges, and more faintly the one under the pointer. */
  highlightEdges(edges: number[], hover: number | null = null): void {
    if (this.edgeHighlight) {
      this.scene.remove(this.edgeHighlight);
      this.edgeHighlight.traverse((o) => {
        if (o instanceof THREE.Line) {
          o.geometry.dispose();
          (o.material as THREE.Material).dispose();
        }
      });
      this.edgeHighlight = null;
    }
    const group = new THREE.Group();
    const draw = (edge: number, color: number) => {
      const line = this.edgeLines[edge];
      if (!line || line.length < 6) return;
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute("position", new THREE.BufferAttribute(line, 3));
      const material = new THREE.LineBasicMaterial({ color, depthTest: false });
      const drawn = new THREE.Line(geometry, material);
      drawn.renderOrder = 10;
      group.add(drawn);
    };
    for (const edge of edges) draw(edge, 0xffb454);
    if (hover !== null && !edges.includes(hover)) draw(hover, 0xffe0b0);
    if (group.children.length > 0) {
      this.edgeHighlight = group;
      this.scene.add(group);
    }
    this.render();
  }

  /**
   * Draws simulated stock in place of the part, coloured by each triangle's
   * class: 0 machined to size, 1 material left, 2 gouged, 3 off the part.
   * `null` hides it and shows the part again.
   */
  setStock(mesh: Mesh | null): void {
    if (this.stock) {
      this.scene.remove(this.stock);
      this.stock.geometry.dispose();
      (this.stock.material as THREE.Material).dispose();
      this.stock = null;
    }
    if (this.model) this.model.visible = mesh === null && !this.hidden.has("bodies");
    if (mesh) {
      const colors = [0x9fb0c2, 0xf6c343, 0xe5484d, 0x5b6b7c].map((c) => new THREE.Color(c));
      const count = mesh.triangleFaces.length;
      const positions = new Float32Array(count * 9);
      const tints = new Float32Array(count * 9);
      for (let t = 0; t < count; t++) {
        const color = colors[mesh.triangleFaces[t] ?? 3] ?? colors[3]!;
        for (let k = 0; k < 3; k++) {
          const v = mesh.indices[3 * t + k] ?? 0;
          for (let a = 0; a < 3; a++) positions[9 * t + 3 * k + a] = mesh.positions[3 * v + a] ?? 0;
          tints.set([color.r, color.g, color.b], 9 * t + 3 * k);
        }
      }
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
      geometry.setAttribute("color", new THREE.BufferAttribute(tints, 3));
      geometry.computeVertexNormals();
      this.stock = new THREE.Mesh(
        geometry,
        new THREE.MeshStandardMaterial({ vertexColors: true, metalness: 0.2, roughness: 0.6, side: THREE.DoubleSide }),
      );
      this.scene.add(this.stock);
    }
    this.render();
  }

  /** Draws a toolpath: cuts solid, rapids dashed; `null` hides it. */
  setToolpath(path: { rapids: Float32Array; cuts: Float32Array } | null): void {
    if (this.toolpath) {
      this.scene.remove(this.toolpath);
      this.toolpath.traverse((o) => {
        if (o instanceof THREE.LineSegments) {
          o.geometry.dispose();
          (o.material as THREE.Material).dispose();
        }
      });
      this.toolpath = null;
    }
    if (path) {
      const group = new THREE.Group();
      const add = (points: Float32Array, material: THREE.LineBasicMaterial | THREE.LineDashedMaterial) => {
        if (points.length === 0) return;
        const geometry = new THREE.BufferGeometry();
        geometry.setAttribute("position", new THREE.BufferAttribute(points, 3));
        const lines = new THREE.LineSegments(geometry, material);
        lines.computeLineDistances();
        lines.renderOrder = 5;
        group.add(lines);
      };
      add(path.cuts, new THREE.LineBasicMaterial({ color: 0x4fd1c5, depthTest: false, transparent: true, opacity: 0.9 }));
      add(path.rapids, new THREE.LineDashedMaterial({ color: 0xf6c343, dashSize: 2, gapSize: 2, depthTest: false, transparent: true, opacity: 0.8 }));
      group.visible = !this.hidden.has("toolpaths");
      this.toolpath = group;
      this.scene.add(group);
    }
    this.render();
  }

  /** The canvas, for pointer events. */
  get canvas(): HTMLCanvasElement {
    return this.renderer.domElement;
  }

  /** Whether a model is shown. */
  get hasModel(): boolean {
    return this.model !== null;
  }

  /** Draws the document's sketches on their planes. */
  setSketches(records: { record: SketchRecord; frame: Frame }[]): void {
    for (const child of [...this.sketches.children]) {
      this.sketches.remove(child);
      if (child instanceof THREE.LineSegments) {
        child.geometry.dispose();
        (child.material as THREE.Material).dispose();
      }
    }
    for (const { record, frame } of records) {
      const { segments, construction } = sketchSegments(record);
      const add = (points: XY[], material: THREE.LineBasicMaterial | THREE.LineDashedMaterial) => {
        if (points.length === 0) return;
        const geometry = new THREE.BufferGeometry().setFromPoints(
          points.map((p) => onFrame(frame, p)),
        );
        const lines = new THREE.LineSegments(geometry, material);
        lines.computeLineDistances();
        this.sketches.add(lines);
      };
      add(segments, new THREE.LineBasicMaterial({ color: 0x8fc1ff }));
      add(construction, new THREE.LineDashedMaterial({ color: 0xc98a4b, dashSize: 2, gapSize: 1.5 }));
    }
    this.render();
  }

  /** Removes the model. */
  clear(): void {
    this.highlightFace(null);
    this.surface = null;
    this.triangleFaces = null;
    if (!this.model) return;
    this.scene.remove(this.model);
    this.model.traverse((object) => {
      if (object instanceof THREE.Mesh || object instanceof THREE.LineSegments) {
        object.geometry.dispose();
        (object.material as THREE.Material).dispose();
      }
    });
    this.model = null;
    this.render();
  }

  /** Shows or hides a layer. */
  setLayer(layer: Layer, visible: boolean): void {
    if (visible) this.hidden.delete(layer);
    else this.hidden.add(layer);
    if (layer === "bodies" && this.model) this.model.visible = visible && this.stock === null;
    if (layer === "sketches") this.sketches.visible = visible;
    if (layer === "toolpaths" && this.toolpath) this.toolpath.visible = visible;
    if (layer === "grid") this.grid.visible = visible;
    this.render();
  }

  /** Whether a layer is shown. */
  isShown(layer: Layer): boolean {
    return !this.hidden.has(layer);
  }

  /** Shaded faces with edges, or edges only. */
  setWireframe(on: boolean): void {
    this.wireframe = on;
    if (this.surface) this.surface.visible = !on;
    this.render();
  }

  /** Looks at the whole model from a standard view. */
  look(view: ViewName): void {
    const { from, up } = VIEWS[view];
    this.fit(new THREE.Vector3(...from).normalize(), new THREE.Vector3(...up));
  }

  /** Points the camera at the whole model, from an isometric-style view
   * unless a direction is given. */
  fit(
    direction = new THREE.Vector3(...VIEWS.home.from).normalize(),
    up = new THREE.Vector3(0, 0, 1),
  ): void {
    const box = new THREE.Box3();
    if (this.model) box.expandByObject(this.model);
    if (this.sketches.children.length > 0) box.expandByObject(this.sketches);
    if (box.isEmpty()) box.set(new THREE.Vector3(-50, -50, 0), new THREE.Vector3(50, 50, 50));
    const sphere = box.getBoundingSphere(new THREE.Sphere());
    const radius = Math.max(sphere.radius, 1);
    const distance = radius / Math.sin(THREE.MathUtils.degToRad(this.camera.fov / 2));
    this.camera.up.copy(up);
    this.camera.position.copy(sphere.center).addScaledVector(direction, distance * 1.1);
    this.camera.near = distance / 100;
    this.camera.far = distance * 100;
    this.camera.updateProjectionMatrix();
    this.controls.target.copy(sphere.center);
    this.controls.update();

    // Size the grid to the part: about four times its footprint.
    const size = Math.max(box.max.x - box.min.x, box.max.y - box.min.y, 10);
    const scale = Math.pow(10, Math.ceil(Math.log10(size * 4))) / 200;
    this.grid.scale.setScalar(scale);
    this.render();
  }

  private resize(): void {
    const { clientWidth: width, clientHeight: height } = this.host;
    if (width === 0 || height === 0) return;
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    this.render();
  }

  private loop = (): void => {
    requestAnimationFrame(this.loop);
    // Damping keeps the camera moving briefly after the mouse stops.
    this.controls.update();
  };

  private render(): void {
    this.renderer.render(this.scene, this.camera);
    this.cube?.render(this.camera, this.controls.target);
  }
}
