// dist/project-tiles-<version>.zip: the mod folder as it goes into ~/.config/herdr-app/mods/.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync } from "node:fs";

const { version } = JSON.parse(readFileSync("package.json", "utf8"));
const out = `dist/project-tiles-${version}.zip`;
mkdirSync("dist", { recursive: true });
rmSync(out, { force: true });
execFileSync("zip", ["-qr", `../../${out}`, "project-tiles", "-x", "*/gpui-kit.d.ts", "*/jsconfig.json", "*/.DS_Store"],
  { cwd: "herdr-app/mods", stdio: "inherit" });
console.log(out);
