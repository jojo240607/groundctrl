// 地面站 Web 前端（Tauri v2）。
// 通过 window.__TAURI__.core.invoke 调用后端命令，通过 listen 接收遥测事件。

import { drawMap } from './map.js';
import { drawAttitude } from './attitude.js';
import { drawGauges } from './gauges.js';
import { drawTrend } from './trend.js';

const { invoke, listen } = window.__TAURI__;

// ---- 全局状态 ----
const state = {
  vehicles: [],
  selected: 0,
  alarms: [],
  params: new Map(),
  mission: [],
  trend: { t: [], alt: [], spd: [], batt: [] },
};

// ---- 工具 ----
const $ = (id) => document.getElementById(id);
const fmt = (v, d = 1) => (v == null || isNaN(v)) ? '--' : Number(v).toFixed(d);

// ---- 后端调用 ----
async function getSettings() {
  try {
    const s = await invoke('get_settings');
    $('conn-bind').value = s.default_url.includes(':') ? s.default_url : '0.0.0.0:14551';
    $('cfg-warn').value = 30;
    $('cfg-crit').value = 15;
    $('cfg-radius').value = 1000;
    $('cfg-lat').value = 31;
    $('cfg-lon').value = 121;
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
    row.className = 'wp-row';
    row.textContent = `#${i} cmd=${w.command} (${fmt(w.x,5)}, ${fmt(w.y,5)}) ${fmt(w.z,1)}m`;
    box.appendChild(row);
  });
}

function selectedVehicle() {
  return state.vehicles.find(v => v.sysid === state.selected) || state.vehicles[0] || null;
}

// ---- 事件监听 ----
async function setupListeners() {
  await listen('fleet', (e) => {
    state.vehicles = e.payload.vehicles;
    state.selected = e.payload.selected || (state.vehicles[0] && state.vehicles[0].sysid) || 0;
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
  // 保留最近 600 点
  const MAX = 600;
  if (tr.t.length > MAX) { tr.t.shift(); tr.alt.shift(); tr.spd.shift(); tr.batt.shift(); }
}

function renderAll() {
  updateStatusLine();
  const v = selectedVehicle();
  drawMap($('map'), state, v);
  drawAttitude($('attitude'), v);
  drawGauges($('gauges'), v);
  drawTrend($('trend'), state.trend);
}

// ---- 绑定 ----
function bindUi() {
  $('btn-connect').addEventListener('click', connect);
  $('btn-disconnect').addEventListener('click', disconnect);
  $('btn-cfg').addEventListener('click', applyCfg);
  $('btn-params').addEventListener('click', fetchParams);
  $('btn-upload-mission').addEventListener('click', uploadMission);
  $('param-filter').addEventListener('input', renderParams);
  // 演示用：点击地图可添加航点（以地图中心为参考）
  $('map').addEventListener('click', (ev) => {
    const rect = ev.target.getBoundingClientRect();
    const v = selectedVehicle();
    const lat = v ? v.lat : 31.0;
    const lon = v ? v.lon : 121.0;
    const dx = (ev.clientX - rect.left) / rect.width - 0.5;
    const dy = (ev.clientY - rect.top) / rect.height - 0.5;
    state.mission.push({
      command: 16,
      x: lat - dy * 0.02,
      y: lon + dx * 0.02,
      z: 50,
    });
    renderMission();
  });
}

// ---- 启动 ----
async function main() {
  bindUi();
  await setupListeners();
  await getSettings();
  // 初始渲染一帧
  renderAll();
  // 若已连接（后端持久），主动拉一次机队
  try { const f = await invoke('get_fleet'); state.vehicles = f.vehicles; state.selected = f.selected; renderAll(); } catch (e) {}
}

main();
