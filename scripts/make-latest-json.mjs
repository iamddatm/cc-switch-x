#!/usr/bin/env node
// 生成 Tauri updater 的 latest.json。签名文件内容是 minisign 文本的 base64，
// 这里先解码核对 trusted comment 绑定了正确版本，再原样嵌入 manifest。

import { existsSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename } from "node:path";

function fail(message) {
  console.error(`::error::${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    if (!key.startsWith("--")) fail(`无法识别的参数：${key}`);
    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      fail(`参数 ${key} 缺少值`);
    }
    args[key.slice(2)] = value;
    index += 1;
  }
  return args;
}

function requireArg(args, name) {
  if (!args[name]) fail(`缺少参数 --${name}`);
  return args[name];
}

const args = parseArgs(process.argv.slice(2));
const installer = requireArg(args, "installer");
const signaturePath = args.signature || `${installer}.sig`;
const tag = requireArg(args, "tag");
const output = requireArg(args, "output");
const version = tag.replace(/^v/, "");
const repo = args.repo || process.env.GITHUB_REPOSITORY || "iamddatm/cc-switch-x";

if (!/^v\d+\.\d+\.\d+$/.test(tag)) fail(`tag 必须是 vX.Y.Z：${tag}`);
if (!existsSync(installer)) fail(`安装包不存在：${installer}`);
if (!existsSync(signaturePath)) fail(`签名文件不存在：${signaturePath}`);

const installerName = basename(installer);
const expectedName = `CC-Switch-X_${version}_x64-setup.exe`;
if (installerName !== expectedName) {
  fail(`安装包文件名必须是 ${expectedName}（当前为 ${installerName}）`);
}

const signature = readFileSync(signaturePath, "utf8").trim();
if (!signature) fail(`签名文件为空：${signaturePath}`);

let signatureText;
try {
  signatureText = Buffer.from(signature, "base64").toString("utf8");
} catch {
  fail("签名文件不是合法的 base64");
}
if (!signatureText.includes("trusted comment:")) {
  fail("签名文件解码后缺少 trusted comment");
}
const signedVersion = signatureText.match(/\tversion:([^\r\n\t]+)/)?.[1]?.trim();
if (signedVersion !== version) {
  fail(
    `签名绑定的版本是 ${signedVersion ?? "未知"}，与 manifest 版本 ${version} 不一致；` +
      "请用 tauri build（CLI >= 2.12）重新签名",
  );
}

const notesFile = args["notes-file"];
let notes = `CC Switch X ${tag}`;
if (notesFile) {
  if (!existsSync(notesFile)) fail(`发布说明不存在：${notesFile}`);
  notes = readFileSync(notesFile, "utf8").trim();
  if (!notes) fail(`发布说明为空：${notesFile}`);
  if (notes.length > 8000) notes = `${notes.slice(0, 8000)}\n…`;
}

const baseUrl =
  args["base-url"] || `https://github.com/${repo}/releases/download/${tag}`;
const assetUrl = `${baseUrl.replace(/\/+$/, "")}/${installerName}`;

const manifest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      signature,
      url: assetUrl,
    },
  },
};

writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
const size = statSync(output).size;
console.log(`已生成 ${output}（${size} 字节）：${assetUrl}`);
