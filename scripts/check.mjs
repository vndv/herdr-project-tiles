// Static checks that need no Herdr: syntax of the mod's scripts, its JSON files, the icons it
// uses, and no personal paths in what is published.
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, existsSync } from "node:fs";

const mod = "herdr-app/mods/project-tiles";
const fail = [];

for (const f of readdirSync(mod).filter((f) => f.endsWith(".js"))) {
  try {
    execFileSync(process.execPath, ["--check", `${mod}/${f}`], { stdio: "pipe" });
  } catch (e) {
    fail.push(`${mod}/${f}: ${e.stderr}`);
  }
}

for (const f of [`${mod}/mod.json`, "herdr-app/keys.json", ".github/badges/skylos.json"]) {
  try {
    JSON.parse(readFileSync(f, "utf8"));
  } catch (e) {
    fail.push(`${f}: ${e.message}`);
  }
}

const main = readFileSync(`${mod}/main.js`, "utf8");
const icons = new Set([...main.matchAll(/this\.icon\(\s*"([\w-]+)"/g)].map((m) => m[1]));
for (const name of icons) if (!existsSync(`${mod}/icons/${name}.svg`)) fail.push(`missing icon: ${mod}/icons/${name}.svg`);

const tracked = execFileSync("git", ["ls-files", "-z"], { encoding: "utf8" }).split("\0").filter(Boolean);
for (const f of tracked.filter((f) => /\.(js|mjs|json|md|ya?ml|toml)$/.test(f))) {
  const text = readFileSync(f, "utf8");
  for (const m of text.matchAll(/\/(?:Users|home)\/([a-z][\w.-]*)/g)) {
    if (!["me", "mex", "runner"].includes(m[1])) fail.push(`${f}: personal path ${m[0]}`);
  }
}

if (fail.length) {
  console.error(fail.join("\n"));
  process.exit(1);
}
console.log(`check passed: ${icons.size} icons, ${tracked.length} tracked files`);
