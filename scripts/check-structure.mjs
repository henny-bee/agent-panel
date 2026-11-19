// Guards the native crates' layout (codebase/module structure refactoring.md):
// no Rust file over 1,000 lines, and no `crate::<module>` reaching a higher layer.
// Run from the repository root: node scripts/check-structure.mjs
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const MAX_LINES = 1000;
// A single function that cannot be split without changing code.
const LONG_FILES = new Set(["crates/monocode-gpui/src/chat/transcript/row_body.rs"]);

// Top-level modules, lowest layer first. A module may use modules listed before it.
const LAYERS = {
  "crates/monocode-gpui/src": [
    "ui", "runtime", "notifications", "editor", "inbox", "git", "terminal", "accounts", "chat",
    "footer", "project", "search", "notes", "automations", "mobile", "monos", "settings", "app",
  ],
  "crates/monocode-core/src": [
    "sys", "files", "vcs", "session", "provider", "project", "integrations", "mono", "store", "automations",
  ],
};

// Known upward references ("from -> to"). They need code changes (events/callbacks),
// not moves. Remove a line once its reference is gone; never add new ones.
const ALLOWED = new Set([
  "monos -> app", "chat -> monos", "chat -> mobile", "editor -> chat", "inbox -> chat",
  "notifications -> chat", "runtime -> git", "search -> app", "ui -> settings", "ui -> editor",
  "session -> provider", "files -> vcs", "provider -> mono", "mono -> store", "session -> store",
  "integrations -> store", "project -> mono", "project -> store", "session -> mono", "vcs -> provider",
  "files -> integrations", "provider -> integrations", "provider -> store", "session -> integrations",
  "session -> project", "vcs -> session",
]);

function rustFiles(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return rustFiles(path);
    return name.endsWith(".rs") ? [path] : [];
  });
}

const problems = [];
for (const [root, order] of Object.entries(LAYERS)) {
  const rank = new Map(order.map((name, index) => [name, index]));
  const modules = new Set(readdirSync(root).filter((n) => n.endsWith(".rs")).map((n) => n.slice(0, -3)));
  for (const name of modules) {
    if (name !== "main" && name !== "lib" && !rank.has(name)) {
      problems.push(`${root}/${name}.rs: new top-level module; add it to LAYERS`);
    }
  }
  for (const path of rustFiles(root)) {
    const file = relative(".", path).split("\\").join("/");
    const text = readFileSync(path, "utf8");
    const lines = text.split("\n").length - (text.endsWith("\n") ? 1 : 0);
    if (lines > MAX_LINES && !LONG_FILES.has(file)) {
      problems.push(`${file}: ${lines} lines (max ${MAX_LINES}); split it into child modules`);
    }
    const from = relative(root, path).split("\\").join("/").split("/")[0].replace(/\.rs$/, "");
    if (!rank.has(from)) continue;
    const used = new Set([...text.matchAll(/crate::(\w+)/g)].map((m) => m[1]));
    for (const group of text.matchAll(/crate::\{([^}]*)\}/g)) {
      for (const m of group[1].matchAll(/(?:^|,)\s*(\w+)/g)) used.add(m[1]);
    }
    for (const to of used) {
      if (to === from || !modules.has(to) || !rank.has(to)) continue;
      if (rank.get(to) > rank.get(from) && !ALLOWED.has(`${from} -> ${to}`)) {
        problems.push(`${file}: ${from} -> ${to} reaches a higher layer`);
      }
    }
  }
}

if (problems.length) {
  console.error(problems.join("\n"));
  process.exit(1);
}
console.log("Native crate structure OK");
