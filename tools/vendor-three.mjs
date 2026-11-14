// Vendor three.js for the demo's 3D desk scene (opendmd-web/share/desk/hd/three.js): run by hand
// when bumping, never by `make site`; commit the output.
//
//     node tools/vendor-three.mjs
//
// Installs three and esbuild at exact versions into a temporary folder, bundles one entry that
// re-exports only the names the scene uses (hd/scene.js: `THREE.*`) and the post-processing it
// needs (EffectComposer, Pass, FullScreenQuad, UnrealBloomPass), minified ESM, and writes it with
// the versions, the licence line and its SHA-256 in a header. The scene stays hand-written modules
// importing ./three.js: no bundler in the site build. A name the scene starts to use must be added
// to NAMES. Served from opendmd.dev itself (script-src 'self'), never a CDN. three.js is MIT
// (licenses/THIRD-PARTY-LICENSES-desk.txt; the site's licences page lists it).
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, rmSync, writeFileSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

// three 0.186.1 (2026-09-24): the newest release at least two weeks old with a patch behind it
const THREE = '0.186.1', ESBUILD = '0.25.10';
const NAMES = ['Box3', 'BoxGeometry', 'CanvasTexture', 'CatmullRomCurve3', 'ClampToEdgeWrapping', 'Color', 'CylinderGeometry', 'DepthTexture',
  'DirectionalLight', 'DoubleSide', 'Group', 'HalfFloatType', 'HemisphereLight', 'LinearFilter', 'LinearMipmapLinearFilter', 'Mesh',
  'MeshBasicMaterial', 'MeshLambertMaterial', 'NearestFilter', 'PerspectiveCamera', 'PlaneGeometry', 'PointLight', 'RepeatWrapping', 'Scene',
  'ShaderMaterial', 'SphereGeometry', 'SpotLight', 'SRGBColorSpace', 'TextureLoader', 'TubeGeometry', 'Vector2', 'Vector3', 'WebGLRenderer',
  'WebGLRenderTarget'];
const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = join(ROOT, 'opendmd-web/share/desk/hd/three.js');
const tmp = mkdtempSync(join(tmpdir(), 'vendor-three-'));
try {
  writeFileSync(join(tmp, 'package.json'), '{"private":true}');
  execFileSync('npm', ['install', '--no-audit', '--no-fund', '--save-exact', `three@${THREE}`, `esbuild@${ESBUILD}`], { cwd: tmp, stdio: 'inherit' });
  const lic = readFileSync(join(tmp, 'node_modules/three/LICENSE'), 'utf8');
  if (!/The MIT License/.test(lic)) throw new Error('three: not the MIT licence any more: read it before vendoring');
  writeFileSync(join(tmp, 'entry.js'), `export { ${NAMES.join(', ')} } from 'three';
export { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
export { Pass, FullScreenQuad } from 'three/addons/postprocessing/Pass.js';
export { UnrealBloomPass } from 'three/addons/postprocessing/UnrealBloomPass.js';
`);
  const { build } = await import(join(tmp, 'node_modules/esbuild/lib/main.js'));
  const r = await build({ entryPoints: [join(tmp, 'entry.js')], bundle: true, minify: true, format: 'esm', target: 'es2022', write: false, legalComments: 'none', absWorkingDir: tmp });
  const body = r.outputFiles[0].text;
  const sha = createHash('sha256').update(body).digest('hex');
  writeFileSync(OUT, `/* three.js ${THREE} (https://threejs.org/), MIT licence, Copyright (c) 2010-2026 three.js authors: licenses/THIRD-PARTY-LICENSES-desk.txt.
   Made by tools/vendor-three.mjs (esbuild ${ESBUILD}; the names opendmd-web/share/desk/hd/scene.js uses, EffectComposer, Pass,
   FullScreenQuad, UnrealBloomPass). sha256 of what follows this comment: ${sha} */\n` + body);
  const all = readFileSync(OUT);
  console.log(`${OUT}: ${all.length} B, ${gzipSync(all, { level: 9 }).length} B gzip`);
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
