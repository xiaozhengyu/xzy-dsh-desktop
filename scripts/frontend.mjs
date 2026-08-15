// 前端资源构建/开发脚本：
//   node scripts/frontend.mjs          → 复制 index.html + src/ → dist/（供 tauri build 使用）
//   node scripts/frontend.mjs --serve  → 复制后启动静态服务器（供 tauri dev 使用，devUrl=1420）
import { cp, mkdir, rm } from "node:fs/promises";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, join, normalize, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dist = join(root, "dist");
const port = Number(process.env.DEV_PORT || 1420);

await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });
await cp(join(root, "index.html"), join(dist, "index.html"));
await cp(join(root, "src"), join(dist, "src"), { recursive: true });
console.log(`[frontend] copied -> ${dist}`);

if (process.argv.includes("--serve")) {
  const MIME = {
    ".html": "text/html; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".svg": "image/svg+xml",
    ".png": "image/png",
    ".json": "application/json; charset=utf-8",
  };
  createServer(async (req, res) => {
    try {
      let pathname = decodeURIComponent(new URL(req.url, "http://localhost").pathname);
      if (pathname === "/") pathname = "/index.html";
      const file = normalize(join(dist, pathname));
      if (!file.startsWith(dist)) { res.writeHead(403); return res.end("forbidden"); }
      const data = await readFile(file);
      res.writeHead(200, { "content-type": MIME[extname(file).toLowerCase()] ?? "application/octet-stream" });
      res.end(data);
    } catch {
      res.writeHead(404);
      res.end("not found");
    }
  }).listen(port, "127.0.0.1", () => console.log(`[frontend] dev server: http://127.0.0.1:${port}`));
}
