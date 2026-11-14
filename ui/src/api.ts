// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The only way the UI talks to the core (architecture rule 5): JSON requests
// through the `api` command, and binary meshes through `step_preview`
// (rule 12). Nothing here knows about the engine or the kernel.

import { invoke } from "@tauri-apps/api/core";

export type Unit = "millimetre" | "degree" | "unitless";

export interface Parameter {
  value: number;
  unit: Unit;
  comment?: string;
}

export interface XY {
  x: number;
  y: number;
}

export type Geometry =
  | { type: "point"; at: XY }
  | { type: "line"; start: number; end: number }
  | { type: "circle"; center: number; radius: number }
  | { type: "arc"; center: number; start: number; end: number };

export interface Entity {
  geometry: Geometry;
  construction?: boolean;
}

/** A constraint as the core stores it: a type plus its references. */
export interface Constraint {
  type: string;
  value?: number;
  [reference: string]: unknown;
}

export interface Sketch {
  entities: Record<string, Entity>;
  constraints: Record<string, Constraint>;
  next_entity: number;
  next_constraint: number;
}

export type Plane = "xy" | "xz" | "yz" | { face: string };

/** Where a sketch sits in the model, in model coordinates. */
export interface Frame {
  origin: Point3;
  x: [number, number, number];
  y: [number, number, number];
  normal: [number, number, number];
}

export type FrameStatus = { state: "ok"; frame: Frame } | { state: "failed"; reason: string };

/** A short label for a plane: XY, XZ, YZ or Face. */
export function planeLabel(plane: Plane): string {
  return typeof plane === "string" ? plane.toUpperCase() : "Face";
}

export interface SketchRecord {
  name: string;
  plane: Plane;
  sketch: Sketch;
  /** Dimensions driven by an expression, by constraint id. */
  expressions?: Record<string, string>;
}

export type FreedomStatus = "under" | "fully" | "over" | "unknown";

export interface Freedom {
  dof: number | null;
  redundant: number;
  status: FreedomStatus;
  entities: { id: number; dof: number | null; status: FreedomStatus }[];
}

export interface HoleSpec {
  diameter: number;
  depth: { blind: number } | "through";
  tip_angle: number;
  counterbore?: { diameter: number; depth: number };
  thread?: { size: string; depth: number };
  fit?: string;
}

export type Feature =
  | {
      type: "extrude";
      name: string;
      sketch: number;
      profiles: number[][];
      distance: number;
      distance_expression?: string;
      operation: "join" | "cut";
    }
  | { type: "hole"; name: string; sketch: number; points: number[]; spec: HoleSpec }
  | { type: "chamfer"; name: string; edges: EdgeTarget[]; distance: number }
  | { type: "fillet"; name: string; edges: EdgeTarget[]; radius: number }
  | {
      type: "pattern";
      name: string;
      feature: number;
      count_x: number;
      spacing_x: number;
      count_y: number;
      spacing_y: number;
    };

export type EdgeTarget = { face: string } | { between: [string, string] };

export type FeatureStatus = { state: "ok" } | { state: "failed"; reason: string };

export interface BodyView {
  volume: number;
  bounds: { min: Point3; max: Point3 };
  faces: string[][];
  /** The two faces each edge joins, by edge number. */
  edges: [number, number][];
}

export type CamOperation =
  | { type: "holes" }
  | { type: "face"; tool?: number }
  | { type: "contour"; face: string; tool?: number; depth?: number }
  | { type: "pocket"; floor: string; tool?: number }
  | { type: "chamfer"; face: string; width: number; tool?: number };

export interface Cam {
  stock?: { side: number; top: number; bottom: number };
  tools?: Record<string, { kind: { type: string }; diameter: number; description: string }>;
  setups?: {
    id: number;
    setup: {
      name: string;
      side: "top" | "bottom";
      work_offset: number;
      flip_about?: "x" | "y";
      index?: number;
      operations: { id: number; operation: CamOperation }[];
    };
  }[];
}

export interface DocumentState {
  document: {
    name: string;
    parameters: Record<string, Parameter>;
    sketches?: Record<string, SketchRecord>;
    features?: { id: number; feature: Feature }[];
    cam?: Cam;
  };
  path: string | null;
  dirty: boolean;
  undo: string | null;
  redo: string | null;
  freedom: Record<string, Freedom>;
  feature_status: Record<string, FeatureStatus>;
  sketch_frames: Record<string, FrameStatus>;
  body: BodyView | null;
  body_revision: number;
  /** How many timeline features are built while the history is rolled back. */
  rollback: number | null;
}

export interface Point3 {
  x: number;
  y: number;
  z: number;
}

export interface StepInfo {
  solids: number;
  shells: number;
  faces: number;
  edges: number;
  vertices: number;
  bounds: { min: Point3; max: Point3 } | null;
}

/** A failed request, with the API's stable error kind. */
export class ApiFailure extends Error {
  constructor(
    readonly kind: string,
    message: string,
    /** For `conflict` and `redundant`: `{constraints: number[]}`. */
    readonly details?: { constraints?: number[] },
  ) {
    super(message);
  }
}

interface Reply<T> {
  ok: boolean;
  result?: T;
  error?: { kind: string; message: string; details?: { constraints?: number[] } };
}

/** Sends one request and returns its result, or throws an ApiFailure. */
export async function request<T>(body: Record<string, unknown>): Promise<T> {
  const text = await invoke<string>("api", { request: JSON.stringify(body) });
  const reply = JSON.parse(text) as Reply<T>;
  if (reply.ok && reply.result !== undefined) {
    return reply.result;
  }
  throw new ApiFailure(
    reply.error?.kind ?? "internal",
    reply.error?.message ?? "the API sent an unreadable reply",
    reply.error?.details,
  );
}

export const state = () => request<DocumentState>({ method: "state" });

export const execute = (command: string, args: Record<string, unknown>) =>
  request<DocumentState>({ method: "execute", command, args });

export interface Mesh {
  positions: Float32Array;
  indices: Uint32Array;
  triangleFaces: Uint32Array;
  faceCount: number;
}

const MAGIC = "CNDTMSH1";

/** Decodes the binary layout documented on `Api::step_preview`. */
export function decodeMesh(buffer: ArrayBuffer): Mesh {
  const view = new DataView(buffer);
  const magic = new TextDecoder().decode(new Uint8Array(buffer, 0, 8));
  if (buffer.byteLength < 20 || magic !== MAGIC) {
    throw new ApiFailure("internal", "the mesh data is not in the expected format");
  }
  const vertices = view.getUint32(8, true);
  const triangles = view.getUint32(12, true);
  const faceCount = view.getUint32(16, true);
  const expected = 20 + 4 * (vertices * 3 + triangles * 4);
  if (buffer.byteLength !== expected) {
    throw new ApiFailure("internal", "the mesh data has the wrong length");
  }
  let offset = 20;
  const positions = new Float32Array(buffer, offset, vertices * 3);
  offset += vertices * 3 * 4;
  const indices = new Uint32Array(buffer, offset, triangles * 3);
  offset += triangles * 3 * 4;
  const triangleFaces = new Uint32Array(buffer, offset, triangles);
  return { positions, indices, triangleFaces, faceCount };
}

/** Triangulates a STEP file in the core and returns the mesh. */
export async function stepPreview(path: string): Promise<Mesh> {
  try {
    const buffer = await invoke<ArrayBuffer>("step_preview", { path });
    return decodeMesh(buffer);
  } catch (error) {
    if (error instanceof ApiFailure) throw error;
    throw new ApiFailure("kernel", String(error));
  }
}

/** Triangulates the rebuilt part in the core and returns the mesh. */
export async function bodyMesh(): Promise<Mesh> {
  try {
    return decodeMesh(await invoke<ArrayBuffer>("body_mesh"));
  } catch (error) {
    if (error instanceof ApiFailure) throw error;
    throw new ApiFailure("kernel", String(error));
  }
}

/** The rebuilt part's edges as polylines (x, y, z per point), by edge number. */
export async function bodyEdges(): Promise<Float32Array[]> {
  let buffer: ArrayBuffer;
  try {
    buffer = await invoke<ArrayBuffer>("body_edges");
  } catch (error) {
    throw new ApiFailure("kernel", String(error));
  }
  const view = new DataView(buffer);
  const magic = new TextDecoder().decode(new Uint8Array(buffer, 0, 8));
  if (buffer.byteLength < 12 || magic !== "CNDTEDG1") {
    throw new ApiFailure("internal", "the edge data is not in the expected format");
  }
  const count = view.getUint32(8, true);
  let offset = 12 + 4 * count;
  if (buffer.byteLength < offset) throw new ApiFailure("internal", "the edge data is too short");
  const lines: Float32Array[] = [];
  for (let i = 0; i < count; i++) {
    const points = view.getUint32(12 + 4 * i, true);
    if (offset + points * 12 > buffer.byteLength) throw new ApiFailure("internal", "the edge data is too short");
    lines.push(new Float32Array(buffer.slice(offset, offset + points * 12)));
    offset += points * 12;
  }
  return lines;
}

/** A setup's toolpath as segments in model coordinates. */
export interface Toolpath {
  rapids: Float32Array;
  cuts: Float32Array;
}

/** A setup's simulated stock, from the last `simulate` of it (decision 0016). */
export async function simulationMesh(setup: number): Promise<Mesh> {
  try {
    return decodeMesh(await invoke<ArrayBuffer>("simulation_mesh", { setup }));
  } catch (error) {
    if (error instanceof ApiFailure) throw error;
    throw new ApiFailure("simulation", String(error));
  }
}

export async function toolpathPreview(setup: number): Promise<Toolpath> {
  let buffer: ArrayBuffer;
  try {
    buffer = await invoke<ArrayBuffer>("toolpath_preview", { setup });
  } catch (error) {
    throw new ApiFailure("cam", String(error));
  }
  const view = new DataView(buffer);
  const magic = new TextDecoder().decode(new Uint8Array(buffer, 0, 8));
  if (buffer.byteLength < 12 || magic !== "CNDTTPH1") {
    throw new ApiFailure("internal", "the toolpath data is not in the expected format");
  }
  const count = view.getUint32(8, true);
  if (buffer.byteLength !== 12 + count * 25) throw new ApiFailure("internal", "the toolpath data has the wrong length");
  const rapids: number[] = [];
  const cuts: number[] = [];
  for (let i = 0; i < count; i++) {
    const at = 12 + i * 25;
    const out = view.getUint8(at) === 0 ? rapids : cuts;
    for (let k = 0; k < 6; k++) out.push(view.getFloat32(at + 1 + 4 * k, true));
  }
  return { rapids: new Float32Array(rapids), cuts: new Float32Array(cuts) };
}
