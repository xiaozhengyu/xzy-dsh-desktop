// views-settings.js —— 设置视图：服务（端口/主机）、启动（自启/自动启动）、
// 外观（深浅主题）、调试（DevTools）、关于
(() => {
  "use strict";
  const DSH = window.DSH;
  const invoke = DSH.invoke;
  const state = DSH.state;
  const $ = (id) => document.getElementById(id);

  let built = false;

  function build() {
    const host = $("settings-content");
    host.innerHTML = `
      <div class="card">
        <div class="card-title">服务</div>
        <div class="settings-section">
          <div class="setting-row">
            <div class="setting-label">
              <span class="setting-name">端口</span>
              <span class="setting-desc">dsh web 服务端口，修改后下次启动服务时生效</span>
            </div>
            <div class="setting-control">
              <input id="set-port" class="setting-input" type="number" min="1" max="65535" />
              <button id="btn-save-port" class="btn">保存</button>
            </div>
          </div>
          <div class="setting-row">
            <div class="setting-label">
              <span class="setting-name">绑定主机</span>
              <span class="setting-desc">留空为回环地址 127.0.0.1</span>
            </div>
            <div class="setting-control">
              <input id="set-host" class="setting-input host" placeholder="127.0.0.1" />
              <button id="btn-save-host" class="btn">保存</button>
            </div>
          </div>
        </div>
      </div>

      <div class="card">
        <div class="card-title">启动</div>
        <div class="settings-section">
          <div class="setting-row">
            <div class="setting-label">
              <span class="setting-name">开机自启</span>
              <span class="setting-desc">Windows 登录时自动启动本应用（注册表 Run 键）</span>
            </div>
            <label class="switch"><input id="set-autostart" type="checkbox" /><span class="track"></span></label>
          </div>
          <div class="setting-row">
            <div class="setting-label">
              <span class="setting-name">启动时自动启动服务</span>
              <span class="setting-desc">本应用启动后自动执行「启动服务」</span>
            </div>
            <label class="switch"><input id="set-autostart-service" type="checkbox" /><span class="track"></span></label>
          </div>
        </div>
      </div>

      <div class="card">
        <div class="card-title">外观</div>
        <div class="settings-section">
          <div class="setting-row">
            <div class="setting-label">
              <span class="setting-name">主题</span>
              <span class="setting-desc">界面配色</span>
            </div>
            <div class="setting-control theme-radio">
              <label><input type="radio" name="theme-mode" value="light" /> 浅色</label>
              <label><input type="radio" name="theme-mode" value="dark" /> 深色</label>
              <label><input type="radio" name="theme-mode" value="system" /> 跟随系统</label>
            </div>
          </div>
        </div>
      </div>

      <div class="card">
        <div class="card-title">调试</div>
        <div class="settings-section">
          <div class="setting-row">
            <div class="setting-label">
              <span class="setting-name">自动打开开发者工具</span>
              <span class="setting-desc">启动时自动打开 DevTools（仅调试用）</span>
            </div>
            <label class="switch"><input id="set-devtools" type="checkbox" /><span class="track"></span></label>
          </div>
        </div>
      </div>

      <div class="card">
        <div class="card-title">关于</div>
        <div class="about-block">
          <span>DSH 桌面端（dsh-desktop）<code id="about-version"></code></span>
          <span>DeepSeek Harness 的 Windows 11 桌面封装：低内存、托盘常驻、不内嵌 Node.js。</span>
          <span>配置文件：<code id="about-config-path"></code></span>
          <span>图标：deepseek-whale-girl-icon（CC BY-NC-SA 4.0，非商用须署名）</span>
        </div>
      </div>
    `;
    built = true;
  }

  function flash(btn, text, ms = 1200) {
    const old = btn.textContent;
    btn.textContent = text;
    btn.disabled = true;
    setTimeout(() => {
      btn.textContent = old;
      btn.disabled = false;
    }, ms);
  }

  function bindEvents() {
    $("btn-save-port").addEventListener("click", async () => {
      const port = parseInt($("set-port").value, 10);
      if (!port || port < 1 || port > 65535) {
        alert("端口需为 1-65535 的整数");
        return;
      }
      try {
        await invoke("set_config", { webPort: port });
        flash($("btn-save-port"), "已保存");
        await DSH.reloadConfig();
      } catch (e) {
        alert("保存失败：" + (e?.message || String(e)));
      }
    });

    $("btn-save-host").addEventListener("click", async () => {
      try {
        await invoke("set_config", { webHost: $("set-host").value });
        flash($("btn-save-host"), "已保存");
        await DSH.reloadConfig();
      } catch (e) {
        alert("保存失败：" + (e?.message || String(e)));
      }
    });

    $("set-autostart").addEventListener("change", async (e) => {
      try {
        await invoke("set_autostart", { enabled: e.target.checked });
      } catch (err) {
        alert("设置失败：" + (err?.message || String(err)));
        e.target.checked = !e.target.checked;
      }
    });

    $("set-autostart-service").addEventListener("change", async (e) => {
      try {
        await invoke("set_config", { autoStart: e.target.checked });
      } catch (err) {
        alert("设置失败：" + (err?.message || String(err)));
        e.target.checked = !e.target.checked;
      }
    });

    // 外观：浅色 / 深色 / 跟随系统
    document.querySelectorAll('input[name="theme-mode"]').forEach((radio) => {
      radio.addEventListener("change", async () => {
        if (!radio.checked) return;
        try {
          await invoke("set_config", { themeMode: radio.value });
          await DSH.applyThemeAll();
        } catch (err) {
          alert("设置失败：" + (err?.message || String(err)));
        }
      });
    });

    $("set-devtools").addEventListener("change", async (e) => {
      try {
        await invoke("set_config", { autoOpenDevtools: e.target.checked });
      } catch (err) {
        alert("设置失败：" + (err?.message || String(err)));
        e.target.checked = !e.target.checked;
      }
    });
  }

  DSH.renderSettings = async () => {
    if (!built) {
      build();
      bindEvents();
    }
    await DSH.reloadConfig();
    const cfg = state.config || {};
    $("set-port").value = cfg.webPort ?? 3081;
    $("set-host").value = cfg.webHost ?? "127.0.0.1";
    $("set-autostart").checked = !!cfg.autostart;
    $("set-autostart-service").checked = !!cfg.autoStart;
    const modeRadio = document.querySelector(`input[name="theme-mode"][value="${cfg.themeMode || "system"}"]`);
    if (modeRadio) modeRadio.checked = true;
    $("set-devtools").checked = !!cfg.autoOpenDevtools;

    $("about-version").textContent = await DSH.getAppVersion();
    try {
      const p = await invoke("get_config_path");
      $("about-config-path").textContent = p || "未知";
    } catch (e) {
      $("about-config-path").textContent = "未知";
    }
  };
})();
