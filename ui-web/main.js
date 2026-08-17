// 地面站 Web 前端（Tauri v2）。
// 通过 window.__TAURI__.core.invoke 调用后端命令，通过 listen 接收遥测事件。

import 'leaflet/dist/leaflet.css';
import { initMap, renderWaypoints, renderVehicles, renderFence, panTo, screenToLatLon } from './map.js';
import { drawAttitude } from './attitude.js';
import { drawGauges } from './gauges.js';
import { drawTrend, trendChannels } from './trend.js';

// ===== 后端桥接 =====
// 在 Tauri 运行时中，invoke/listen 通过 @tauri-apps/api 注入的全局对象提供。
// 在无头浏览器(纯 Web)联调环境下，window.__TAURI__ 不存在，改为通过 WebSocket
// 连接本地桥服务器 (gc_bridge.js)，复用同一套 invoke/listen 语义。
let invoke, listen;
if (window.__TAURI__) {
  ({ invoke, listen } = window.__TAURI__.core);
} else {
  // ---- 浏览器 fallback：WebSocket 桥 ----
  const Bridge = (() => {
    const WS_URL = 'ws://localhost:8787';
    let ws = null;
    let reqId = 0;
    const pending = new Map();
    const listeners = new Map(); // event -> Set<cb>
    function ensure() {
      if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) return;
      ws = new WebSocket(WS_URL);
      ws.onmessage = (ev) => {
        const msg = JSON.parse(ev.data);
        if (msg.type === 'resp') {
          const p = pending.get(msg.id);
          if (p) { pending.delete(msg.id); msg.ok ? p.resolve(msg.data) : p.reject(new Error(msg.error)); }
        } else if (msg.type === 'event') {
          const set = listeners.get(msg.event);
          if (set) set.forEach((cb) => cb({ event: msg.event, payload: msg.payload }));
        }
      };
    }
    ensure();
    return {
      invoke(cmd, args) {
        ensure();
        const id = ++reqId;
        return new Promise((resolve, reject) => {
          pending.set(id, { resolve, reject });
          const send = () => ws.send(JSON.stringify({ type: 'invoke', id, cmd, args: args || {} }));
          if (ws.readyState === WebSocket.OPEN) send();
          else ws.addEventListener('open', send, { once: true });
        });
      },
      listen(event, cb) {
        if (!listeners.has(event)) listeners.set(event, new Set());
        listeners.get(event).add(cb);
        return Promise.resolve(() => listeners.get(event).delete(cb));
      },
    };
  })();
  invoke = Bridge.invoke;
  listen = Bridge.listen;
}

// ---- 全局状态 ----
const state = {
  vehicles: [],
  selected: 0,
  alarms: [],
  params: new Map(),
  mission: [],
  mapCenter: { lat: 31.0, lon: 121.0 },
  trail: [],
  trend: { t: [], alt: [], spd: [], batt: [], air: [] },
  visibleChannels: ['alt', 'spd', 'batt'],
  selectedWp: -1,
  didPan: false,
};

// ---- 工具 ----
const $ = (id) => document.getElementById(id);
const fmt = (v, d = 1) => (v == null || isNaN(v)) ? '--' : Number(v).toFixed(d);

// ---- 后端调用 ----
async function getSettings() {
  try {
    const s = await invoke('get_settings');
    $('conn-bind').value = s.default_url.includes(':') ? s.default_url : '0.0.0.0:14551';
    await refreshCfg();
  } catch (e) { console.warn('get_settings', e); }
}

async function refreshCfg() {
  try {
    const c = await invoke('get_monitor_config');
    $('cfg-warn').value = c.batteryWarnPct;
    $('cfg-crit').value = c.batteryCriticalPct;
    $('cfg-radius').value = c.fenceRadiusM;
    $('cfg-lat').value = c.fenceLat;
    $('cfg-lon').value = c.fenceLon;
  } catch (e) { console.warn('get_monitor_config', e); }
}

async function connect() {
  const kind = $('conn-kind').value;
  const bind = $('conn-bind').value;
  const target = $('conn-target').value;
  $('btn-connect').disabled = true;
  try {
    await invoke('connect', { args: { kind, bind, target } });
    setLink('已连接', true);
  } catch (e) {
    alert('连接失败：' + e);
  } finally {
    $('btn-connect').disabled = false;
  }
}

async function disconnect() {
  await invoke('disconnect');
  setLink('未连接', false);
}

async function applyCfg() {
  const cfg = {
    batteryWarnPct: Number($('cfg-warn').value),
    batteryCriticalPct: Number($('cfg-crit').value),
    fenceRadiusM: Number($('cfg-radius').value),
    fenceLat: Number($('cfg-lat').value),
    fenceLon: Number($('cfg-lon').value),
  };
  await invoke('set_monitor_config', { cfg });
  window.__FENCE__ = { lat: cfg.fenceLat, lon: cfg.fenceLon, radius: cfg.fenceRadiusM };
}

async function fetchParams() {
  const v = selectedVehicle();
  if (!v) { alert('尚未连接或无可用的飞行器'); return; }
  $('params').innerHTML = '<div class="muted">拉取中...</div>';
  state.params.clear();
  try {
    await invoke('request_params', { sys: v.sysid, comp: v.compid });
  } catch (e) { alert('拉取参数失败：' + e); }
}

async function setParam(name, value) {
  const v = selectedVehicle();
  if (!v) return;
  await invoke('set_param', { sys: v.sysid, comp: v.compid, name, value });
}

async function uploadMission() {
  const v = selectedVehicle();
  if (!v) { alert('尚未连接'); return; }
  await invoke('upload_mission', {
    sys: v.sysid, comp: v.compid,
    items: state.mission.map((w, i) => ({
      seq: i, command: w.command || 16, x: w.x, y: w.y, z: w.z, autocontinue: true,
    })),
  });
}

async function downloadMission() {
  const v = selectedVehicle();
  if (!v) { alert('尚未连接'); return; }
  try {
    const m = await invoke('download_mission', { sys: v.sysid, comp: v.compid });
    if (m && m.items && m.items.length) {
      state.mission = m.items.map((it) => ({
        command: it.command ?? 16, x: it.x, y: it.y, z: it.z,
      }));
      renderMission();
      renderMap();
    } else {
      alert('飞行器无航点');
    }
  } catch (e) {
    console.warn('download_mission', e);
    alert('下载航点失败：' + e);
  }
}

// ---- 状态栏 ----
function setLink(text, ok) {
  const el = $('st-link');
  el.textContent = '链路：' + text;
  el.style.color = ok ? 'var(--good)' : 'var(--muted)';
}

function updateStatusLine() {
  const v = selectedVehicle();
  $('st-mode').textContent = '模式：' + (v ? v.flightMode : '--');
  $('st-arm').textContent = '状态：' + (v ? (v.armed ? '已解锁' : '已上锁') : '--');
  $('st-batt').textContent = '电量：' + (v && v.battery != null ? v.battery.toFixed(0) + '%' : '--');
  $('st-alt').textContent = '高度：' + (v && v.altRel != null ? fmt(v.altRel, 1) + ' m' : '--');
  $('st-spd').textContent = '地速：' + (v && v.groundSpeed != null ? fmt(v.groundSpeed, 1) + ' m/s' : '--');
}

function renderAlarms() {
  const box = $('alarms');
  if (state.alarms.length === 0) { box.innerHTML = '<div class="muted">暂无告警</div>'; return; }
  box.innerHTML = '';
  for (const a of state.alarms.slice(-30).reverse()) {
    const div = document.createElement('div');
    div.className = 'alarm-item ' + a.level;
    div.innerHTML = `<div>${a.message}</div><div class="meta">${a.code} · ${a.link}</div>`;
    box.appendChild(div);
  }
}

function renderParams() {
  const box = $('params');
  const f = $('param-filter').value.trim().toLowerCase();
  const rows = [...state.params.values()].filter(p => !f || p.name.toLowerCase().includes(f));
  if (rows.length === 0) { box.innerHTML = '<div class="muted">无匹配参数</div>'; return; }
  box.innerHTML = '';
  for (const p of rows) {
    const row = document.createElement('div');
    row.className = 'param-row';
    row.innerHTML = `<span class="pname" title="双击写入">${p.name}</span><span class="pval">${fmt(p.value, 3)}</span>`;
    row.querySelector('.pname').addEventListener('dblclick', async () => {
      const nv = prompt('写入参数 ' + p.name + ' 的新值：', p.value);
      if (nv != null) { await setParam(p.name, parseFloat(nv)); }
    });
    box.appendChild(row);
  }
}

function renderMission() {
  const box = $('mission');
  if (state.mission.length === 0) { box.innerHTML = '<div class="muted">无航点</div>'; return; }
  box.innerHTML = '';
  state.mission.forEach((w, i) => {
    const row = document.createElement('div');
    row.className = 'wp-row' + (i === state.selectedWp ? ' sel' : '');
    row.innerHTML = `
      <span class="wp-idx">${i}</span>
      <span class="wp-coord" title="点击在地图上选中">
        ${fmt(w.x, 5)}, ${fmt(w.y, 5)}
      </span>
      <input class="wp-z" type="number" title="高度(m)" value="${fmt(w.z, 0)}" />
      <span class="wp-act">
        <button class="up" title="上移">↑</button>
        <button class="down" title="下移">↓</button>
        <button class="del" title="删除">✕</button>
      </span>`;
    // 选中
    row.querySelector('.wp-coord').addEventListener('click', () => {
      state.selectedWp = i; renderMission(); renderMap();
    });
    // 高度编辑
    row.querySelector('.wp-z').addEventListener('change', (e) => {
      w.z = parseFloat(e.target.value) || 0;
    });
    // 上移
    row.querySelector('.up').addEventListener('click', () => {
      if (i > 0) { [state.mission[i - 1], state.mission[i]] = [state.mission[i], state.mission[i - 1]]; state.selectedWp = i - 1; renderMission(); renderMap(); }
    });
    // 下移
    row.querySelector('.down').addEventListener('click', () => {
      if (i < state.mission.length - 1) { [state.mission[i + 1], state.mission[i]] = [state.mission[i], state.mission[i + 1]]; state.selectedWp = i + 1; renderMission(); renderMap(); }
    });
    // 删除
    row.querySelector('.del').addEventListener('click', () => {
      state.mission.splice(i, 1);
      if (state.selectedWp >= state.mission.length) state.selectedWp = state.mission.length - 1;
      renderMission(); renderMap();
    });
    box.appendChild(row);
  });
}

function selectedVehicle() {
  return state.vehicles.find(v => v.sysid === state.selected) || state.vehicles[0] || null;
}

function mapCenterV() {
  const v = selectedVehicle();
  return v && v.lat ? { lat: v.lat, lon: v.lon } : state.mapCenter;
}

// ---- 事件监听 ----
async function setupListeners() {
  await listen('fleet', (e) => {
    const prev = state.vehicles;
    state.vehicles = e.payload.vehicles;
    state.selected = e.payload.selected || (state.vehicles[0] && state.vehicles[0].sysid) || 0;
    // 记录航迹
    const v = selectedVehicle();
    if (v && v.lat) {
      state.mapCenter = { lat: v.lat, lon: v.lon };
      if (!state.didPan) { panTo(v.lat, v.lon); state.didPan = true; }
      const last = state.trail[state.trail.length - 1];
      if (!last || Math.hypot(last.lat - v.lat, last.lon - v.lon) > 1e-5) {
        state.trail.push({ lat: v.lat, lon: v.lon });
        if (state.trail.length > 300) state.trail.shift();
      }
    }
    updateStatusLine();
    pushTrend();
    renderAll();
  });
  await listen('alarm', (e) => {
    state.alarms.push(e.payload);
    if (state.alarms.length > 200) state.alarms.shift();
    renderAlarms();
  });
  await listen('link-state', (e) => {
    setLink(e.payload.connected ? '已连接' : '未连接', e.payload.connected);
  });
  await listen('param-value', (e) => {
    state.params.set(e.payload.name, e.payload);
    renderParams();
  });
  await listen('params-progress', (e) => {
    $('params').innerHTML = `<div class="muted">拉取中 ${e.payload.received}/${e.payload.expected}</div>`;
  });
}

function pushTrend() {
  const v = selectedVehicle();
  const t = Date.now();
  const tr = state.trend;
  tr.t.push(t);
  tr.alt.push(v ? v.altRel ?? 0 : 0);
  tr.spd.push(v ? v.groundSpeed ?? 0 : 0);
  tr.batt.push(v ? v.battery ?? 0 : 0);
  tr.air.push(v ? v.airSpeed ?? 0 : 0);
  const MAX = 600;
  if (tr.t.length > MAX) { tr.t.shift(); tr.alt.shift(); tr.spd.shift(); tr.batt.shift(); tr.air.shift(); }
}

function renderMap() {
  syncMap();
  const c = mapCenterV();
  $('map-readout').textContent = `中心：${c.lat.toFixed(5)}, ${c.lon.toFixed(5)}`;
}

// 把航点/飞机/围栏同步到 Leaflet 图层
function syncMap() {
  renderWaypoints(state.mission, state.selectedWp, wpCallbacks());
  renderVehicles(state.vehicles, state.selected);
  renderFence(window.__FENCE__);
}

// 航点图层回调
function wpCallbacks() {
  return {
    onSelect: (i) => { state.selectedWp = i; renderMission(); },
    onWaypointMove: (i, lat, lng) => {
      if (state.mission[i]) state.mission[i] = { ...state.mission[i], x: lat, y: lng };
    },
    onWaypointMoved: (i) => { renderMission(); },
    onWaypointDelete: (i) => { deleteWp(i); },
  };
}

// 删除指定航点
function deleteWp(i) {
  if (i < 0 || i >= state.mission.length) return;
  state.mission.splice(i, 1);
  if (state.selectedWp >= state.mission.length) state.selectedWp = state.mission.length - 1;
  renderMission(); renderMap();
}

function renderAll() {
  updateStatusLine();
  renderMap();
  drawAttitude($('attitude'), selectedVehicle());
  drawGauges($('gauges'), selectedVehicle());
  drawTrend($('trend'), state.trend, state.visibleChannels);
}

// ---- 航点交互（Leaflet）----
function setupMapInteraction() {
  initMap($('map-leaf'), {
    onCursor: (lat, lon) => {
      $('map-readout').textContent = `坐标：${lat.toFixed(5)}, ${lon.toFixed(5)}`;
    },
    onWaypointAdd: (lat, lon) => {
      state.mission.push({ command: 16, x: lat, y: lon, z: 50 });
      state.selectedWp = state.mission.length - 1;
      renderMission(); renderMap();
    },
    onWaypointDeleteAt: (lat, lon) => {
      // 删除 30m 内最近航点
      let best = -1, bestD = 30;
      state.mission.forEach((wp, i) => {
        const d = Math.hypot(
          (wp.x - lat) * 111320,
          (wp.y - lon) * 111320 * Math.cos(lat * Math.PI / 180),
        );
        if (d < bestD) { bestD = d; best = i; }
      });
      if (best >= 0) deleteWp(best);
    },
    ...wpCallbacks(),
  });
}

// ---- 遥测通道切换 ----
function setupTrendChannels() {
  const box = $('trend-channels');
  box.innerHTML = '';
  for (const c of trendChannels()) {
    const id = 'ch-' + c.key;
    const label = document.createElement('label');
    label.innerHTML = `<input type="checkbox" id="${id}" ${state.visibleChannels.includes(c.key) ? 'checked' : ''}/>
      <span style="color:${c.color}">${c.label}</span>`;
    label.querySelector('input').addEventListener('change', (e) => {
      if (e.target.checked) {
        if (!state.visibleChannels.includes(c.key)) state.visibleChannels.push(c.key);
      } else {
        state.visibleChannels = state.visibleChannels.filter(k => k !== c.key);
      }
      drawTrend($('trend'), state.trend, state.visibleChannels);
    });
    box.appendChild(label);
  }
}

// 串口连接：枚举可用串口并填充下拉，选中后把端口名写入 conn-bind
// （后端 Serial 分支用 bind 字段作为端口名）。
async function refreshSerialPorts() {
  const sel = $('conn-port');
  if (!sel) return;
  try {
    const ports = await invoke('list_serial_ports');
    sel.innerHTML = '';
    if (!ports.length) {
      sel.innerHTML = '<option value="">无可用串口</option>';
    } else {
      for (const p of ports) {
        const o = document.createElement('option');
        o.value = p; o.textContent = p;
        sel.appendChild(o);
      }
    }
    // 把当前选中端口同步到 conn-bind（后端 Serial 取 bind 作为端口名）
    $('conn-bind').value = sel.value || '';
  } catch (e) {
    console.warn('list_serial_ports', e);
  }
}

function onConnKindChange() {
  const kind = $('conn-kind').value;
  const isSerial = kind === 'serial';
  const row = $('row-serial');
  if (row) row.style.display = isSerial ? '' : 'none';
  if (isSerial) refreshSerialPorts();
  if (kind === 'udp') {
    $('conn-bind').placeholder = '绑定地址';
    $('conn-target').placeholder = '目标地址 (UDP)';
  } else if (kind === 'serial') {
    $('conn-bind').placeholder = '串口名 (自动填充)';
    $('conn-target').placeholder = '波特率 (如 115200)';
  } else {
    $('conn-bind').placeholder = '绑定地址';
    $('conn-target').placeholder = '目标地址';
  }
}

// ---- 绑定 ----
function bindUi() {
  $('btn-connect').addEventListener('click', connect);
  $('btn-disconnect').addEventListener('click', disconnect);
  $('btn-cfg').addEventListener('click', applyCfg);
  $('btn-params').addEventListener('click', fetchParams);
  $('btn-upload-mission').addEventListener('click', uploadMission);
  $('btn-mission-upload').addEventListener('click', uploadMission);
  $('btn-wp-download').addEventListener('click', downloadMission);
  $('btn-wp-clear').addEventListener('click', clearMission);
  $('btn-wp-clear2').addEventListener('click', clearMission);
  $('param-filter').addEventListener('input', renderParams);
  $('conn-kind').addEventListener('change', onConnKindChange);
  $('conn-port').addEventListener('change', () => { $('conn-bind').value = $('conn-port').value; });
  $('btn-refresh-port').addEventListener('click', refreshSerialPorts);
  setupMapInteraction();
}

function clearMission() {
  state.mission = [];
  state.selectedWp = -1;
  renderMission();
  renderMap();
}

// ---- 启动 ----
async function main() {
  bindUi();
  onConnKindChange();
  setupTrendChannels();
  await setupListeners();
  await getSettings();
  renderAll();
  try {
    const f = await invoke('get_fleet');
    state.vehicles = f.vehicles;
    state.selected = f.selected;
    renderAll();
  } catch (e) {}
}

main();
