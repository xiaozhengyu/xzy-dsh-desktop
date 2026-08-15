// 本地 crates.io 镜像代理（仅用于在 schannel 不可用的沙箱环境中验证构建）：
//   - /crates-io-index/*        → https://index.crates.io/*（sparse index，config.json 被改写为本地 dl）
//   - /api/v1/crates/{name}/{version}/download → https://static.crates.io/crates/{name}/{name}-{version}.crate
// 特性：keep-alive 连接复用、60s 超时 + 一次重试、响应缓冲后转发。
// 用法：node scripts/crates-mirror.mjs [port]  （默认 7980）
import { createServer } from "node:http";
import { request as httpsRequest } from "node:https";

const PORT = Number(process.argv[2] || process.env.CRATES_MIRROR_PORT || 7980);
const INDEX_UPSTREAM = "index.crates.io";
const STATIC_UPSTREAM = "static.crates.io";
const UPSTREAM_TIMEOUT_MS = 60_000;

const agent = new (await import("node:https")).Agent({ keepAlive: true, maxSockets: 12, timeout: UPSTREAM_TIMEOUT_MS });

function fetchUpstream(targetUrl, headers) {
  return new Promise((resolve, reject) => {
    const u = new URL(targetUrl);
    const req = httpsRequest(
      {
        protocol: u.protocol,
        hostname: u.hostname,
        port: u.port || 443,
        path: u.pathname + u.search,
        method: "GET",
        headers: { ...headers, host: u.host, connection: "keep-alive" },
        agent,
      },
      (res) => resolve(res)
    );
    req.setTimeout(UPSTREAM_TIMEOUT_MS, () => req.destroy(new Error("upstream timeout")));
    req.on("error", reject);
    req.end();
  });
}

/** 拉取上游并返回 { status, headers, body }（跟随重定向，缓冲正文）。 */
async function fetchBody(targetUrl, headers, redirects = 0) {
  const res = await fetchUpstream(targetUrl, headers);
  const status = res.statusCode;
  if (status >= 300 && status < 400 && res.headers.location && redirects < 5) {
    res.resume();
    return fetchBody(new URL(res.headers.location, targetUrl).toString(), headers, redirects + 1);
  }
  const chunks = [];
  for await (const c of res) chunks.push(c);
  return { status, headers: res.headers, body: Buffer.concat(chunks) };
}

createServer(async (req, res) => {
  const pathname = decodeURIComponent(new URL(req.url, "http://x").pathname);
  const reqHeaders = { accept: req.headers.accept, "user-agent": req.headers["user-agent"] };
  try {
    // 1) sparse index 的 config.json → 返回改写后的配置（dl 指向本地代理）
    if (pathname === "/crates-io-index/config.json") {
      const body = Buffer.from(
        JSON.stringify({ dl: `http://127.0.0.1:${PORT}/api/v1/crates`, api: "https://crates.io/" })
      );
      res.writeHead(200, { "content-type": "application/json", "content-length": body.length });
      res.end(body);
      return;
    }

    let upstream;
    // 2) sparse index 文件
    if (pathname.startsWith("/crates-io-index/")) {
      upstream = `https://${INDEX_UPSTREAM}/${pathname.slice("/crates-io-index/".length)}`;
    }
    // 3) crate 下载
    else {
      const m = pathname.match(/^\/api\/v1\/crates\/([^/]+)\/([^/]+)\/download$/);
      if (m) {
        const [, name, version] = m;
        upstream = `https://${STATIC_UPSTREAM}/crates/${name}/${name}-${version}.crate`;
      }
    }

    if (!upstream) {
      res.writeHead(404, { "content-type": "text/plain" });
      res.end("not found");
      return;
    }

    let out = null;
    for (let attempt = 0; attempt < 2 && !out; attempt++) {
      try {
        out = await fetchBody(upstream, reqHeaders);
      } catch (e) {
        if (attempt === 1) throw e;
        console.error(`[crates-mirror] retry ${upstream}: ${e.message}`);
      }
    }
    const outHeaders = { ...out.headers, connection: "keep-alive" };
    delete outHeaders["transfer-encoding"];
    delete outHeaders["content-length"];
    res.writeHead(out.status, outHeaders);
    res.end(out.body);
  } catch (e) {
    console.error(`[crates-mirror] error for ${pathname}: ${e.message}`);
    res.writeHead(502, { "content-type": "text/plain" });
    res.end("proxy error: " + e.message);
  }
}).listen(PORT, "127.0.0.1", () => {
  console.log(`[crates-mirror] http://127.0.0.1:${PORT} (index + downloads)`);
});
