// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The 3D viewport. Z is up, as on a machine tool. Units are millimetres.

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { Mesh } from "./api";

export class Viewer {
  private readonly renderer: THREE.WebGLRenderer;
  private readonly scene = new THREE.Scene();
  private readonly camera = new THREE.PerspectiveCamera(40, 1, 0.1, 100_000);
  private readonly controls: OrbitControls;
  private readonly grid: THREE.GridHelper;
  private model: THREE.Group | null = null;

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

    // GridHelper lies in XZ; turn it into the XY plane.
    this.grid = new THREE.GridHelper(200, 20, 0x3a4149, 0x262b31);
    this.grid.rotation.x = Math.PI / 2;
    this.scene.add(this.grid);
    this.scene.add(new THREE.AxesHelper(20));

    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;
    this.controls.addEventListener("change", () => this.render());

    new ResizeObserver(() => this.resize()).observe(host);
    this.resize();
    this.loop();
  }

  /** Shows a mesh, replacing anything shown before, and frames it. */
  show(mesh: Mesh): void {
    this.clear();
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
    this.model = new THREE.Group();
    this.model.add(surface, edges);
    this.scene.add(this.model);
    this.fit();
  }

  /** Removes the model. */
  clear(): void {
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

  /** Points the camera at the whole model from an isometric-style view. */
  fit(): void {
    const box = this.model
      ? new THREE.Box3().setFromObject(this.model)
      : new THREE.Box3(new THREE.Vector3(-50, -50, 0), new THREE.Vector3(50, 50, 50));
    const sphere = box.getBoundingSphere(new THREE.Sphere());
    const radius = Math.max(sphere.radius, 1);
    const distance = radius / Math.sin(THREE.MathUtils.degToRad(this.camera.fov / 2));
    const direction = new THREE.Vector3(1, -1.4, 1).normalize();
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
  }
}
