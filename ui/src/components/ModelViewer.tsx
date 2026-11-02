import { useEffect, useRef, useState } from 'react';
import { LuRotateCcw, LuBox } from 'react-icons/lu';
import { call } from '../ipc';
import { getTheme } from '../theme';

interface MeshData { triangles: number; shown: number; reduced: boolean; min: number[]; max: number[]; data: string }

const VS = `
attribute vec3 p; attribute vec3 n;
uniform mat4 mvp; uniform mat4 model;
varying vec3 vn;
void main(){ vn = mat3(model) * n; gl_Position = mvp * vec4(p,1.0); }`;
const FS = `
precision mediump float;
varying vec3 vn; uniform vec3 color;
void main(){
  vec3 N = normalize(vn);
  if (!gl_FrontFacing) N = -N; // simplified/odd-wound meshes: light both sides
  float key = abs(dot(N, normalize(vec3(0.4,0.8,0.6))));
  float fill = abs(dot(N, normalize(vec3(-0.6,0.2,-0.5)))) * 0.35;
  float rim = pow(1.0 - abs(N.z), 3.0) * 0.15;
  gl_FragColor = vec4(color * (0.22 + 0.7*key + fill) + rim, 1.0);
}`;

type M4 = Float32Array;
const persp = (fovy: number, a: number, n: number, f: number): M4 => {
  const t = 1 / Math.tan(fovy / 2);
  return new Float32Array([t / a, 0, 0, 0, 0, t, 0, 0, 0, 0, (f + n) / (n - f), -1, 0, 0, (2 * f * n) / (n - f), 0]);
};
const mul = (a: M4, b: M4): M4 => {
  const o = new Float32Array(16);
  for (let i = 0; i < 4; i++) for (let j = 0; j < 4; j++) { let s = 0; for (let k = 0; k < 4; k++) s += a[k * 4 + j] * b[i * 4 + k]; o[i * 4 + j] = s; }
  return o;
};
const rotX = (r: number): M4 => { const c = Math.cos(r), s = Math.sin(r); return new Float32Array([1, 0, 0, 0, 0, c, s, 0, 0, -s, c, 0, 0, 0, 0, 1]); };
const rotY = (r: number): M4 => { const c = Math.cos(r), s = Math.sin(r); return new Float32Array([c, 0, -s, 0, 0, 1, 0, 0, s, 0, c, 0, 0, 0, 0, 1]); };
const trans = (x: number, y: number, z: number): M4 => new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, x, y, z, 1]);
const scale = (k: number): M4 => new Float32Array([k, 0, 0, 0, 0, k, 0, 0, 0, 0, k, 0, 0, 0, 0, 1]);

const hex = (h: string): [number, number, number] => {
  const m = /^#?([0-9a-f]{6})$/i.exec(h.trim());
  if (!m) return [0.6, 0.6, 0.7];
  const n = parseInt(m[1], 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
};

/** WebGL preview of STL / OBJ / 3MF meshes: drag to rotate, wheel to zoom, double-click to reset. */
export function ModelViewer({ fileId, size, auto }: { fileId: string; size: number; auto: boolean }) {
  const [want, setWant] = useState(auto);
  const [mesh, setMesh] = useState<MeshData | null>(null);
  const [err, setErr] = useState('');
  const canvas = useRef<HTMLCanvasElement>(null);
  const view = useRef({ yaw: -0.6, pitch: 0.45, zoom: 1 });
  const redraw = useRef<() => void>(() => {});

  useEffect(() => {
    if (!want) return;
    let alive = true;
    setMesh(null); setErr('');
    call<MeshData>('files.mesh', { id: fileId }).then((m) => alive && setMesh(m), (e) => alive && setErr(e.message));
    return () => { alive = false; };
  }, [fileId, want]);

  useEffect(() => {
    const cv = canvas.current;
    if (!mesh || !cv) return;
    const gl = cv.getContext('webgl', { antialias: true, alpha: true });
    if (!gl) { setErr('3D preview needs WebGL, which is not available here.'); return; }
    const bin = atob(mesh.data);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const pos = new Float32Array(bytes.buffer);
    // Z-up (STL/3MF) → Y-up view; flat normals per triangle
    const P = new Float32Array(pos.length), N = new Float32Array(pos.length);
    for (let i = 0; i < pos.length; i += 3) { P[i] = pos[i]; P[i + 1] = pos[i + 2]; P[i + 2] = -pos[i + 1]; }
    for (let t = 0; t < P.length; t += 9) {
      const ax = P[t + 3] - P[t], ay = P[t + 4] - P[t + 1], az = P[t + 5] - P[t + 2];
      const bx = P[t + 6] - P[t], by = P[t + 7] - P[t + 1], bz = P[t + 8] - P[t + 2];
      let nx = ay * bz - az * by, ny = az * bx - ax * bz, nz = ax * by - ay * bx;
      const l = Math.hypot(nx, ny, nz) || 1; nx /= l; ny /= l; nz /= l;
      for (let k = 0; k < 3; k++) { N[t + k * 3] = nx; N[t + k * 3 + 1] = ny; N[t + k * 3 + 2] = nz; }
    }
    const sh = (type: number, src: string) => { const s = gl.createShader(type)!; gl.shaderSource(s, src); gl.compileShader(s); return s; };
    const prog = gl.createProgram()!;
    gl.attachShader(prog, sh(gl.VERTEX_SHADER, VS)); gl.attachShader(prog, sh(gl.FRAGMENT_SHADER, FS)); gl.linkProgram(prog); gl.useProgram(prog);
    const buf = (data: Float32Array, name: string) => {
      const b = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b); gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      const loc = gl.getAttribLocation(prog, name); gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, 3, gl.FLOAT, false, 0, 0);
      return b;
    };
    const b1 = buf(P, 'p'), b2 = buf(N, 'n');
    const [mnx, mny, mnz] = mesh.min, [mxx, mxy, mxz] = mesh.max;
    const cx = (mnx + mxx) / 2, cy = (mnz + mxz) / 2, cz = -(mny + mxy) / 2;
    const radius = Math.max(mxx - mnx, mxy - mny, mxz - mnz) / 2 || 1;
    const uMvp = gl.getUniformLocation(prog, 'mvp'), uModel = gl.getUniformLocation(prog, 'model'), uColor = gl.getUniformLocation(prog, 'color');
    gl.enable(gl.DEPTH_TEST);
    const draw = () => {
      const dpr = window.devicePixelRatio || 1;
      const w = cv.clientWidth, h = cv.clientHeight;
      if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) { cv.width = Math.round(w * dpr); cv.height = Math.round(h * dpr); }
      gl.viewport(0, 0, cv.width, cv.height);
      gl.clearColor(0, 0, 0, 0); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      const v = view.current;
      const model = mul(mul(rotX(v.pitch), rotY(v.yaw)), mul(scale(1 / radius), trans(-cx, -cy, -cz)));
      const mvp = mul(mul(persp(0.75, w / Math.max(h, 1), 0.05, 50), trans(0, 0, -2.9 / v.zoom)), model);
      gl.uniformMatrix4fv(uMvp, false, mvp); gl.uniformMatrix4fv(uModel, false, model);
      gl.uniform3fv(uColor, hex(getTheme().a1));
      gl.drawArrays(gl.TRIANGLES, 0, P.length / 3);
    };
    redraw.current = draw;
    draw();
    const ro = new ResizeObserver(draw); ro.observe(cv);
    const onTheme = () => draw();
    window.addEventListener('pv-theme', onTheme);
    return () => { ro.disconnect(); window.removeEventListener('pv-theme', onTheme); gl.deleteBuffer(b1); gl.deleteBuffer(b2); gl.deleteProgram(prog); };
  }, [mesh]);

  const drag = useRef<{ x: number; y: number } | null>(null);
  const reset = () => { view.current = { yaw: -0.6, pitch: 0.45, zoom: 1 }; redraw.current(); };

  if (!want) return (
    <div className="modelview empty"><button className="btn" onClick={() => setWant(true)}><LuBox /> Show 3D preview</button><span className="small dim">{(size / 1048576).toFixed(0)} MB model — loading may take a few seconds</span></div>
  );
  if (err) return <div className="small muted">{err}</div>;
  const dims = mesh ? [mesh.max[0] - mesh.min[0], mesh.max[1] - mesh.min[1], mesh.max[2] - mesh.min[2]].map((x) => x.toFixed(1)).join(' × ') : '';
  return (
    <div className="modelview">
      {!mesh && <div className="mv-loading small muted">Loading model…</div>}
      <canvas ref={canvas} tabIndex={0} aria-label="3D model preview. Drag or use the arrow keys to rotate, scroll or +/- to zoom."
        onPointerDown={(e) => { drag.current = { x: e.clientX, y: e.clientY }; (e.target as HTMLElement).setPointerCapture(e.pointerId); }}
        onPointerMove={(e) => { if (!drag.current) return; const v = view.current; v.yaw += (e.clientX - drag.current.x) * 0.01; v.pitch = Math.max(-1.5, Math.min(1.5, v.pitch + (e.clientY - drag.current.y) * 0.01)); drag.current = { x: e.clientX, y: e.clientY }; redraw.current(); }}
        onPointerUp={() => { drag.current = null; }}
        onWheel={(e) => { view.current.zoom = Math.max(0.3, Math.min(8, view.current.zoom * (e.deltaY < 0 ? 1.1 : 0.9))); redraw.current(); }}
        onDoubleClick={reset}
        onKeyDown={(e) => {
          const v = view.current; const k = e.key;
          if (k === 'ArrowLeft') v.yaw -= 0.15; else if (k === 'ArrowRight') v.yaw += 0.15; else if (k === 'ArrowUp') v.pitch = Math.max(-1.5, v.pitch - 0.15); else if (k === 'ArrowDown') v.pitch = Math.min(1.5, v.pitch + 0.15);
          else if (k === '+' || k === '=') v.zoom = Math.min(8, v.zoom * 1.15); else if (k === '-') v.zoom = Math.max(0.3, v.zoom / 1.15); else if (k === '0') return reset(); else return;
          e.preventDefault(); redraw.current();
        }} />
      {mesh && <div className="mv-bar small">
        <span className="dim">{dims} mm · {mesh.triangles.toLocaleString()} triangles{mesh.reduced ? ` (showing ${mesh.shown.toLocaleString()})` : ''}</span>
        <button className="btn ghost sm icon right" title="Reset view (double-click)" onClick={reset}><LuRotateCcw /></button>
      </div>}
    </div>
  );
}
