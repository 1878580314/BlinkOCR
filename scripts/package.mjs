import {
  copyFile,
  mkdir,
  readdir,
  readFile,
  writeFile,
  cp,
  rm,
  stat,
} from "node:fs/promises";
import { createReadStream } from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const executable = process.argv[2];
if (!executable)
  throw new Error(
    "Usage: node scripts/package.mjs <built-exe> [output-directory]",
  );
const runtime = path.join(root, ".cache/runtime");
const metadata = JSON.parse(
  await readFile(path.join(runtime, "runtime.json"), "utf8").then((text) =>
    text.replace(/^\uFEFF/, ""),
  ),
);
const output = path.resolve(
  process.argv[3] ??
    path.join(
      root,
      `dist/BlinkOCR-0.1-windows-x64-${metadata.backend.toLowerCase()}`,
    ),
);
const stage = output + ".tmp";
await rm(stage, { recursive: true, force: true });
await mkdir(path.join(stage, "models"), { recursive: true });
await copyFile(executable, path.join(stage, "BlinkOCR.exe"));
await copyFile(
  path.join(path.dirname(executable), "DirectML.dll"),
  path.join(stage, "DirectML.dll"),
);
await cp(path.join(runtime, "bin"), path.join(stage, "bin"), {
  recursive: true,
});
await cp(path.join(root, "licenses"), path.join(stage, "licenses"), {
  recursive: true,
});
await cp(path.join(runtime, "licenses"), path.join(stage, "licenses/runtime"), {
  recursive: true,
});
for (const name of [
  "LICENSE",
  "THIRD_PARTY_NOTICES.md",
  "README.md",
  "README.en.md",
])
  await copyFile(path.join(root, name), path.join(stage, name));
await copyFile(
  path.join(root, "scripts/config.rust.toml"),
  path.join(stage, "config.rust.toml"),
);
await copyFile(
  path.join(root, "scripts/便携版说明.txt"),
  path.join(stage, "PORTABLE.zh-CN.txt"),
);
const screenshots = path.join(root, "assets/screenshots");
if (
  await stat(screenshots)
    .then(() => true)
    .catch(() => false)
)
  await cp(screenshots, path.join(stage, "assets/screenshots"), {
    recursive: true,
  });
const hashes = [];
async function hashTree(directory, relative = "") {
  for (const name of (await readdir(directory)).sort()) {
    const file = path.join(directory, name);
    const local = path.posix.join(relative, name);
    if ((await stat(file)).isDirectory()) await hashTree(file, local);
    else if (local !== "config.rust.toml") {
      const hash = createHash("sha256");
      for await (const chunk of createReadStream(file)) hash.update(chunk);
      hashes.push(`${hash.digest("hex")}  ${local}`);
    }
  }
}
await hashTree(stage);
await writeFile(path.join(stage, "SHA256SUMS"), hashes.join("\n") + "\n");
// This command creates a distribution directory, not an in-place updater.
if (
  await stat(output)
    .then(() => true)
    .catch(() => false)
)
  throw new Error(
    `Output already exists; choose a new distribution directory: ${output}`,
  );
await (await import("node:fs/promises")).rename(stage, output);
console.log(output);
