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

export interface DocumentState {
  document: { name: string; parameters: Record<string, Parameter> };
  path: string | null;
  dirty: boolean;
  undo: string | null;
  redo: string | null;
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
  ) {
    super(message);
  }
}

interface Reply<T> {
  ok: boolean;
  result?: T;
  error?: { kind: string; message: string };
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
