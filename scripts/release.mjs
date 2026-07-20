// Cut a release. Every run bumps the version (patch by default; pass `minor`
// or `major`, or an exact `x.y.z`), verifies the build + tests pass locally,
// then commits, tags, and pushes. Pushing the `v*` tag triggers the GitHub
// Actions release workflow, which rebuilds on macOS/Windows/Linux and
// publishes the release once all platform builds and tests succeed. On
// macOS it then also builds a local .dmg for immediate testing, overwriting
// `release/Jarvis.dmg` each run (not accumulating per-version files).
//
//   pnpm release            # 0.1.0 -> 0.1.1
//   pnpm release minor      # 0.1.1 -> 0.2.0
//   pnpm release major      # 0.2.0 -> 1.0.0
//   pnpm release 1.4.2      # -> exactly 1.4.2
//
// Set SKIP_RELEASE_CHECKS=1 to skip the local build/test gate (not recommended).
// Set SKIP_LOCAL_DMG=1 to skip the local macOS dmg build.

import { readFileSync, writeFileSync, mkdirSync, copyFileSync, readdirSync } from "node:fs";
import { execSync } from "node:child_process";

const run = (cmd, opts = {}) =>
  execSync(cmd, { stdio: "inherit", ...opts });
const capture = (cmd) => execSync(cmd, { encoding: "utf8" }).trim();

// --- 0. Safety: on main, clean working tree ------------------------------
const branch = capture("git rev-parse --abbrev-ref HEAD");
if (branch !== "main") {
  console.error(`✗ releases must be cut from 'main' (currently on '${branch}').`);
  process.exit(1);
}
if (capture("git status --porcelain")) {
  console.error("✗ working tree is dirty — commit or stash changes first.");
  process.exit(1);
}

// --- 1. Compute next version ---------------------------------------------
const arg = process.argv[2] ?? "patch";
const pkgPath = "package.json";
const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
const [maj, min, pat] = pkg.version.split(".").map(Number);

let next;
if (/^\d+\.\d+\.\d+$/.test(arg)) {
  next = arg; // exact version, e.g. `pnpm release 1.4.2`
} else if (["patch", "minor", "major"].includes(arg)) {
  next =
    arg === "major"
      ? `${maj + 1}.0.0`
      : arg === "minor"
        ? `${maj}.${min + 1}.0`
        : `${maj}.${min}.${pat + 1}`;
} else {
  console.error(`✗ unknown arg '${arg}' (use patch | minor | major | an exact x.y.z).`);
  process.exit(1);
}
const tag = `v${next}`;

console.log(`\n▶ Releasing ${tag}  (was v${pkg.version})\n`);

// --- 2. Write the version into all three manifests -----------------------
pkg.version = next;
writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");

const confPath = "src-tauri/tauri.conf.json";
const conf = JSON.parse(readFileSync(confPath, "utf8"));
conf.version = next;
writeFileSync(confPath, JSON.stringify(conf, null, 2) + "\n");

const cargoPath = "src-tauri/Cargo.toml";
const cargo = readFileSync(cargoPath, "utf8").replace(
  /^version = "\d+\.\d+\.\d+"/m,
  `version = "${next}"`,
);
writeFileSync(cargoPath, cargo);

// Keep Cargo.lock in sync with the new package version.
run("cargo update -p jarvis --manifest-path src-tauri/Cargo.toml --offline || true");

// --- 3. Local gate: build + test BEFORE tagging --------------------------
if (!process.env.SKIP_RELEASE_CHECKS) {
  console.log("\n▶ Verifying build + tests before tagging…\n");
  run("pnpm install --frozen-lockfile");
  run("node scripts/fetch-models.mjs");
  run("pnpm build"); // tsc + vite
  run("cargo test --manifest-path src-tauri/Cargo.toml");
} else {
  console.log("⚠ SKIP_RELEASE_CHECKS set — skipping local gate.");
}

// --- 4. Commit, tag, push ------------------------------------------------
run(
  `git add ${pkgPath} ${confPath} ${cargoPath} src-tauri/Cargo.lock`,
);
run(`git commit -m "release: ${tag}"`);
run(`git tag -a ${tag} -m "Jarvis ${tag}"`);
run("git push origin main");
run(`git push origin ${tag}`);

console.log(`\n✓ Pushed ${tag}. GitHub Actions will build + publish the release.`);
console.log("  Watch: https://github.com/Nxe5/jarvis-voice/actions\n");

// --- 5. Local macOS build for immediate testing ---------------------------
// The CI build is the one that actually ships; this is just a same-version
// local artifact so you don't have to wait on CI to install and test it.
// Always overwrites `release/Jarvis.dmg` — never accumulates old versions.
if (process.platform === "darwin" && !process.env.SKIP_LOCAL_DMG) {
  console.log(`▶ Building local macOS .dmg for ${tag}…\n`);
  try {
    run("pnpm tauri build");
    const dmgDir = "src-tauri/target/release/bundle/dmg";
    const dmgFile = readdirSync(dmgDir).find((f) => f.endsWith(".dmg"));
    if (!dmgFile) throw new Error(`no .dmg found in ${dmgDir}`);
    mkdirSync("release", { recursive: true });
    copyFileSync(`${dmgDir}/${dmgFile}`, "release/Jarvis.dmg");
    console.log(`\n✓ Local build ready: release/Jarvis.dmg (${tag})\n`);
  } catch (e) {
    console.error(`\n⚠ Local dmg build failed (release was still published): ${e.message}\n`);
  }
}
