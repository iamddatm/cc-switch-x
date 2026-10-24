#!/usr/bin/env node
// 发布门禁：版本号、发布说明和 updater 配置必须同时成立，避免构建出无法更新的安装包。

import { appendFileSync, existsSync, readFileSync, statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repo = process.env.GITHUB_REPOSITORY || "iamddatm/cc-switch-x";

function fail(message) {
  console.error(`::error::${message}`);
  process.exit(1);
}

function read(path) {
  return readFileSync(resolve(root, path), "utf8");
}

function readJson(path) {
  try {
    return JSON.parse(read(path));
  } catch (error) {
    fail(`${path} 不是合法 JSON：${error.message}`);
  }
}

function cargoPackageVersion(source) {
  const match = source.match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m);
  if (!match) fail("src-tauri/Cargo.toml 缺少 [package].version");
  return match[1];
}

function cargoLockVersion(source) {
  const match = source.match(
    /\[\[package\]\]\s*\nname = "cc-switch-x"\s*\nversion = "([^"]+)"/,
  );
  if (!match) fail("src-tauri/Cargo.lock 里找不到 cc-switch-x 的版本");
  return match[1];
}

const explicitTag = process.argv[2]?.trim();
const isTag = explicitTag ? true : process.env.GITHUB_REF_TYPE === "tag";
const tag = explicitTag || (isTag ? process.env.GITHUB_REF_NAME : null);

if (tag && !/^v\d+\.\d+\.\d+$/.test(tag)) {
  fail(`发布 tag 必须是 vX.Y.Z（当前为 ${tag}）`);
}

const packageJson = readJson("package.json");
const tauriConf = readJson("src-tauri/tauri.conf.json");
const versions = {
  "package.json": packageJson.version,
  "src-tauri/tauri.conf.json": tauriConf.version,
  "src-tauri/Cargo.toml": cargoPackageVersion(read("src-tauri/Cargo.toml")),
  "src-tauri/Cargo.lock": cargoLockVersion(read("src-tauri/Cargo.lock")),
};

const version = packageJson.version;
for (const [file, value] of Object.entries(versions)) {
  if (value !== version) {
    fail(`版本不一致：package.json=${version}，${file}=${value}`);
  }
}

if (tag && tag.slice(1) !== version) {
  fail(`tag ${tag} 与版本 ${version} 不一致`);
}

if (isTag) {
  const notesPath = resolve(root, "docs/release-notes", `${tag}.md`);
  if (!existsSync(notesPath) || statSync(notesPath).size === 0) {
    fail(`缺少发布说明 docs/release-notes/${tag}.md`);
  }
}

const bundle = tauriConf.bundle ?? {};
if (bundle.active !== true) fail("bundle.active 必须为 true");
if (bundle.createUpdaterArtifacts !== true) {
  fail("bundle.createUpdaterArtifacts 必须为 true");
}
const targets = Array.isArray(bundle.targets) ? bundle.targets : [bundle.targets];
if (!targets.includes("nsis")) fail("bundle.targets 必须包含 nsis");
if (bundle.windows?.nsis?.installMode !== "currentUser") {
  fail('bundle.windows.nsis.installMode 必须为 "currentUser"');
}
if (!Array.isArray(bundle.icon) || bundle.icon.length === 0) {
  fail("bundle.icon 不能为空");
}
for (const icon of bundle.icon) {
  if (!existsSync(resolve(root, "src-tauri", icon))) {
    fail(`bundle.icon 指向的文件不存在：${icon}`);
  }
}

const updater = tauriConf.plugins?.updater;
if (!updater) fail("缺少 plugins.updater 配置");
if (typeof updater.pubkey !== "string" || updater.pubkey.length === 0) {
  fail("plugins.updater.pubkey 不能为空");
}
const decodedPubkey = Buffer.from(updater.pubkey, "base64").toString("utf8");
if (!decodedPubkey.includes("minisign public key")) {
  fail("plugins.updater.pubkey 不是合法的 minisign 公钥");
}
if (updater.requireSignedVersion !== true) {
  fail("plugins.updater.requireSignedVersion 必须为 true");
}
for (const flag of [
  "dangerousInsecureTransportProtocol",
  "dangerousAcceptInvalidCerts",
  "dangerousAcceptInvalidHostnames",
]) {
  if (updater[flag] === true) fail(`plugins.updater.${flag} 不允许开启`);
}

const endpoints = updater.endpoints;
if (!Array.isArray(endpoints) || endpoints.length === 0) {
  fail("plugins.updater.endpoints 不能为空");
}
for (const endpoint of endpoints) {
  let url;
  try {
    url = new URL(endpoint);
  } catch {
    fail(`endpoint 不是合法 URL：${endpoint}`);
  }
  if (url.protocol !== "https:") fail(`endpoint 必须使用 HTTPS：${endpoint}`);
  if (url.hostname !== "github.com" || !url.pathname.startsWith(`/${repo}/releases/`)) {
    fail(`endpoint 必须指向 ${repo} 的 GitHub Releases：${endpoint}`);
  }
}

if (process.env.GITHUB_ENV) {
  appendFileSync(
    process.env.GITHUB_ENV,
    `VERSION=${version}\nTAG=${tag ?? `v${version}`}\n`,
  );
}

console.log(`版本与发布配置检查通过：v${version}`);
