// Copies the files the server serves (see crates/xau-web/src/scramjet.rs) out of the installed
// npm packages into the folder XAU_SCRAMJET_DIR points at, with their licences beside them.
//
//   npm ci --ignore-scripts && node layout.mjs <folder>
import { cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const out = process.argv[2];
if (!out) {
  console.error('usage: node layout.mjs <folder>');
  process.exit(2);
}
const pkg = (name) => join('node_modules', '@mercuryworkshop', name);
const files = [
  [join(pkg('scramjet-controller'), 'dist/controller.api.js'), 'controller/controller.api.js'],
  [join(pkg('scramjet-controller'), 'dist/controller.inject.js'), 'controller/controller.inject.js'],
  [join(pkg('scramjet-controller'), 'dist/controller.sw.js'), 'controller/controller.sw.js'],
  [join(pkg('scramjet'), 'dist/scramjet.js'), 'scram/scramjet.js'],
  [join(pkg('scramjet'), 'dist/scramjet.wasm'), 'scram/scramjet.wasm'],
  [join(pkg('libcurl-transport'), 'dist/index.js'), 'clients/libcurl-client.js'],
];
for (const [from, to] of files) {
  if (!existsSync(from)) {
    console.error(`missing ${from}: run npm ci first`);
    process.exit(1);
  }
  mkdirSync(join(out, to, '..'), { recursive: true });
  cpSync(from, join(out, to));
}

// ---- patches to Scramjet itself (AGPL-3.0: this file is where they are published) ----------------
//
// Each one is checked to apply exactly once, so a version bump that moves the code fails the build
// here instead of shipping a page that silently misbehaves.
const patches = [
  {
    file: 'scram/scramjet.js',
    what: 'history.pushState/replaceState without a URL must leave the URL alone',
    why:
      "Microsoft's sign-in page calls history.replaceState(state, '') (no URL). 2.0.67-alpha.2 turns the missing " +
      "argument into the text 'undefined' and rewrites it, so the page's address becomes /undefined and everything " +
      "it reads from its own address (the flow's parameters) and resolves against it is wrong. Fixed upstream after " +
      'this release; this is the same fix.',
    // `s=String(args[2])` in the History.prototype.pushState/replaceState wrapper
    find: /(box\.histories\.get\((\w+)\.this\),(\w+)=)(\(0,\w+\.\w+\))\(\2\.args\[2\]\)/g,
    replace: (_m, head, t, _s, str) => `${head}void 0===${t}.args[2]||null===${t}.args[2]?void 0:${str}(${t}.args[2])`,
  },
];
const applied = [];
for (const p of patches) {
  const path = join(out, p.file);
  const src = readFileSync(path, 'utf8');
  let n = 0;
  const patched = src.replace(p.find, (...a) => (n++, p.replace(...a)));
  if (n !== 1) {
    console.error(`patch "${p.what}": expected to apply once, applied ${n} times (did Scramjet change?)`);
    process.exit(1);
  }
  writeFileSync(path, `${patched}\n/* patched by xau-web (docker/scramjet/layout.mjs): ${p.what} */\n`);
  applied.push(`${p.file}: ${p.what}\n  ${p.why}`);
}
// The sources are AGPL-3.0: keep the licence and where the source is next to the files.
const notes = ['scramjet', 'scramjet-controller', 'libcurl-transport'].map((n) => {
  const p = JSON.parse(readFileSync(join(pkg(n), 'package.json'), 'utf8'));
  return `${p.name}@${p.version} (${p.license}) ${typeof p.repository === 'string' ? p.repository : p.repository?.url ?? ''}`;
});
mkdirSync(out, { recursive: true });
writeFileSync(
  join(out, 'SOURCES.txt'),
  `These files are served by xau-web. Their sources:\n${notes.join('\n')}\n\nChanges made to them (the code of each is in docker/scramjet/layout.mjs of xau-web):\n${applied.join('\n')}\n`,
);
console.log(`ok: ${files.length} files, ${applied.length} patch(es), in ${out}`);
