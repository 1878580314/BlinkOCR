import { readFile, readdir, stat, writeFile, mkdir } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const overrides = JSON.parse(
  await readFile(
    path.join(root, "licenses/license-overrides.json"),
    "utf8",
  ).then((text) => text.replace(/^\uFEFF/, "")),
);
const output = path.join(root, "licenses");
await mkdir(output, { recursive: true });
async function texts(directory, extra = []) {
  const files = new Set(extra.map((file) => path.resolve(directory, file)));
  for (const name of await readdir(directory))
    if (/^(licen[cs]e|copying|copyright|notice)(?:$|[-_.s])/i.test(name)) {
      const file = path.join(directory, name);
      if ((await stat(file)).isFile()) files.add(file);
      else
        for (const child of await readdir(file))
          if ((await stat(path.join(file, child))).isFile())
            files.add(path.join(file, child));
    }
  const result = [];
  for (const file of files) {
    if (file !== directory && !file.startsWith(directory + path.sep)) continue;
    result.push({
      name: path.relative(directory, file),
      text: await readFile(file, "utf8"),
    });
  }
  return result;
}
function document(packages) {
  return packages
    .map(
      (pkg) =>
        `${"=".repeat(72)}\n${pkg.name} ${pkg.version}\nLicense: ${pkg.license}\nSource: ${pkg.source}\n${pkg.source_archive ? `Source archive: ${pkg.source_archive}\n` : ""}${pkg.notices.map((item) => `\n--- ${item.name} ---\n${item.text}`).join("\n")}`,
    )
    .join("\n\n");
}
const metadata = JSON.parse(
  execFileSync(
    "cargo",
    [
      "metadata",
      "--locked",
      "--format-version",
      "1",
      "--filter-platform",
      "x86_64-pc-windows-msvc",
      "--manifest-path",
      path.join(root, "src-tauri/Cargo.toml"),
    ],
    { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 },
  ),
);
const nodes = new Map(metadata.resolve.nodes.map((node) => [node.id, node]));
const visited = new Set();
function walk(id) {
  if (visited.has(id)) return;
  visited.add(id);
  for (const dep of nodes.get(id)?.deps ?? [])
    if (dep.dep_kinds.some((kind) => kind.kind !== "dev")) walk(dep.pkg);
}
walk(metadata.resolve.root);
const rust = [];
const missing = [];
for (const pkg of metadata.packages.filter(
  (pkg) => visited.has(pkg.id) && pkg.id !== metadata.resolve.root,
)) {
  let notices = await texts(
    path.dirname(pkg.manifest_path),
    pkg.license_file ? [pkg.license_file] : [],
  );
  if (!notices.length && overrides[`${pkg.name}@${pkg.version}`]) {
    const source = overrides[`${pkg.name}@${pkg.version}`];
    notices = [
      {
        name: source.source,
        text: await readFile(path.join(output, source.file), "utf8"),
      },
    ];
  }
  if (!notices.length) missing.push(`${pkg.name} ${pkg.version}`);
  rust.push({
    source_archive: `https://crates.io/api/v1/crates/${pkg.name}/${pkg.version}/download`,
    authors: pkg.authors,
    name: pkg.name,
    version: pkg.version,
    license: pkg.license ?? "See package notices",
    source:
      pkg.repository ?? `https://crates.io/crates/${pkg.name}/${pkg.version}`,
    notices,
  });
}
rust.sort(
  (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version),
);
const lock = JSON.parse(
  await readFile(path.join(root, "package-lock.json"), "utf8"),
);
const npm = [];
for (const [location, entry] of Object.entries(lock.packages)) {
  if (!location || entry.dev) continue;
  const dir = path.join(root, location);
  const pkg = JSON.parse(
    await readFile(path.join(dir, "package.json"), "utf8"),
  );
  let notices = await texts(dir);
  if (!notices.length && overrides[`${pkg.name}@${pkg.version}`]) {
    const source = overrides[`${pkg.name}@${pkg.version}`];
    notices = [
      {
        name: source.source,
        text: await readFile(path.join(output, source.file), "utf8"),
      },
    ];
  }
  if (!notices.length) missing.push(`${pkg.name} ${pkg.version}`);
  npm.push({
    name: pkg.name,
    version: pkg.version,
    license: entry.license ?? pkg.license ?? "See package notices",
    source:
      typeof pkg.repository === "string"
        ? pkg.repository
        : (pkg.repository?.url ??
          `https://www.npmjs.com/package/${pkg.name}/v/${pkg.version}`),
    notices,
  });
}
npm.sort((a, b) => a.name.localeCompare(b.name));
await writeFile(
  path.join(output, "RUST-DEPENDENCIES.txt"),
  document(rust) + "\n",
);
await writeFile(
  path.join(output, "NPM-DEPENDENCIES.txt"),
  document(npm) + "\n",
);
await writeFile(
  path.join(output, "dependencies.json"),
  JSON.stringify(
    {
      rust: rust.map(({ notices, ...pkg }) => pkg),
      npm: npm.map(({ notices, ...pkg }) => pkg),
    },
    null,
    2,
  ) + "\n",
);
if (missing.length)
  throw new Error(`Missing license files: ${missing.join(", ")}`);
console.log(
  `Collected licenses: ${rust.length} Rust packages, ${npm.length} npm packages`,
);
