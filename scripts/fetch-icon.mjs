// 下载鲸鱼娘图标仓库的资源（https://github.com/fornarwhal/deepseek-whale-girl-icon）
// 用法：node scripts/fetch-icon.mjs
import { get as httpsGet } from "node:https";
import { writeFileSync, mkdirSync } from "node:fs";

const BASE = "https://raw.githubusercontent.com/fornarwhal/deepseek-whale-girl-icon/main/";
const FILES = ["improved-1.png", "whale-girl-transparent.png", "DeepSeekHarness-WhaleGirl.ico"];

function get(url, timeoutMs = 60000) {
  return new Promise((resolve, reject) => {
    const req = httpsGet(url, { headers: { "User-Agent": "dsh-desktop-icon-fetch" } }, (r) => {
      if (r.statusCode >= 300 && r.statusCode < 400 && r.headers.location) {
        r.resume();
        return resolve(get(new URL(r.headers.location, url).toString(), timeoutMs));
      }
      if (r.statusCode !== 200) {
        r.resume();
        return reject(new Error(`HTTP ${r.statusCode} for ${url}`));
      }
      const chunks = [];
      r.on("data", (c) => chunks.push(c));
      r.on("end", () => resolve(Buffer.concat(chunks)));
    });
    req.setTimeout(timeoutMs, () => req.destroy(new Error(`timeout ${url}`)));
    req.on("error", reject);
  });
}

mkdirSync("assets", { recursive: true });
for (const name of FILES) {
  for (let attempt = 1; attempt <= 3; attempt++) {
    try {
      const body = await get(BASE + name);
      writeFileSync(`assets/${name}`, body);
      console.log(`OK  ${name}  ${body.length} bytes`);
      break;
    } catch (e) {
      if (attempt === 3) console.log(`FAIL ${name}: ${e.message}`);
      else console.log(`retry ${name} (${attempt}): ${e.message}`);
    }
  }
}
