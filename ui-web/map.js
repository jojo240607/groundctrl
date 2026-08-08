// 态势地图：以选中飞机（或地图中心）为图心，等距圆柱投影，绘制经纬度刻度、
// 比例尺、指北针、围栏、航迹与航点。支持鼠标拖拽平移、滚轮缩放、航点拖拽编辑。
//
// 设计取舍：纯手绘 Canvas2D，零外部依赖、完全离线可用（真实地面站现场常无网络，
// 在线瓦片地图不可靠），因此不引入 Leaflet/OSM 瓦片。

const view = { scale: 1, ox: 0, oy: 0 };

// 每度纬度约 111.32 km，用于把米换算成度
const M_PER_DEG = 111320;

// 屏幕坐标 <-> 经纬度 互转（基于当前视图与画布尺寸）
export function screenToLatLon(canvas, px, py, center, span) {
  const w = canvas.width, h = canvas.height;
  const dx = ((px - w / 2 - view.ox) / (w / 2 * view.scale)) * span;
  const dy = ((py - h / 2 - view.oy) / (w / 2 * view.scale)) * span;
  return { lat: center.lat + dy, lon: center.lon + dx };
}

export function getMapView() { return view; }
export function setMapView(v) { Object.assign(view, v); }

// 投影：经纬度 -> 屏幕像素
function project(lat, lon, w, h, center, span) {
  const dx = (lon - center.lon) / span;
  const dy = (lat - center.lat) / span;
  return {
    x: w / 2 + dx * (w / 2) * view.scale + view.ox,
    y: h / 2 - dy * (h / 2) * view.scale + view.oy,
  };
}

function niceSpan(span) {
  // 返回一个"好看"的跨度（用于刻度），基于 1-2-5 进制
  const pow = Math.pow(10, Math.floor(Math.log10(span)));
  const f = span / pow;
  const step = f < 1.5 ? 1 : f < 3.5 ? 2 : f < 7.5 ? 5 : 10;
  return step * pow;
}

export function drawMap(canvas, state, v, opts = {}) {
  const ctx = canvas.getContext('2d');
  const w = canvas.width, h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = '#0a0d12';
  ctx.fillRect(0, 0, w, h);

  const center = v && v.lat ? { lat: v.lat, lon: v.lon } : (state.mapCenter || { lat: 31.0, lon: 121.0 });
  const span = 0.03 / view.scale; // 半宽（度），默认约 3km

  drawGraticule(ctx, w, h, center, span);
  drawScaleBar(ctx, w, h, center, span);
  drawCompass(ctx, w, h);

  // 围栏
  if (window.__FENCE__) {
    const f = window.__FENCE__;
    const p = project(f.lat, f.lon, w, h, center, span);
    const r = (f.radius / M_PER_DEG) / span * (w / 2) * view.scale;
    ctx.strokeStyle = 'rgba(210,153,34,0.7)';
    ctx.setLineDash([5, 4]);
    ctx.beginPath(); ctx.arc(p.x, p.y, r, 0, Math.PI * 2); ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = 'rgba(210,153,34,0.9)';
    ctx.font = '10px sans-serif';
    ctx.fillText('围栏', p.x + r * 0.7 + 2, p.y - r * 0.7);
  }

  // 航迹（基于趋势里的位置采样，可选）
  if (state.trail && state.trail.length > 1) {
    ctx.strokeStyle = 'rgba(47,129,247,0.5)';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    state.trail.forEach((pt, i) => {
      const p = project(pt.lat, pt.lon, w, h, center, span);
      if (i === 0) ctx.moveTo(p.x, p.y); else ctx.lineTo(p.x, p.y);
    });
    ctx.stroke();
  }

  // 航点连线 + 节点
  const mission = state.mission || [];
  if (mission.length > 0) {
    ctx.strokeStyle = 'rgba(63,185,80,0.8)';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    mission.forEach((wp, i) => {
      const p = project(wp.x, wp.y, w, h, center, span);
      if (i === 0) ctx.moveTo(p.x, p.y); else ctx.lineTo(p.x, p.y);
    });
    ctx.stroke();
    mission.forEach((wp, i) => {
      const p = project(wp.x, wp.y, w, h, center, span);
      const sel = opts.selectedWp === i;
      ctx.fillStyle = sel ? '#f0a500' : '#3fb950';
      ctx.beginPath(); ctx.arc(p.x, p.y, sel ? 7 : 5, 0, Math.PI * 2); ctx.fill();
      ctx.fillStyle = '#cdd9e5';
      ctx.font = '10px sans-serif';
      ctx.fillText(String(i), p.x + 8, p.y - 6);
    });
  }

  // 飞机
  for (const veh of state.vehicles) {
    if (!veh.lat) continue;
    const p = project(veh.lat, veh.lon, w, h, center, span);
    drawPlane(ctx, p.x, p.y, veh.yaw || 0, veh === v);
    ctx.fillStyle = '#9fb3c8';
    ctx.font = '11px sans-serif';
    ctx.fillText(veh.flightMode || '', p.x + 12, p.y - 12);
  }

  // 中心十字
  ctx.strokeStyle = '#243040';
  ctx.beginPath(); ctx.moveTo(w / 2 - 6, h / 2); ctx.lineTo(w / 2 + 6, h / 2); ctx.stroke();
  ctx.beginPath(); ctx.moveTo(w / 2, h / 2 - 6); ctx.lineTo(w / 2, h / 2 + 6); ctx.stroke();

  // 坐标读数（鼠标位置）
  if (opts.cursor && opts.cursor.inside) {
    const ll = screenToLatLon(canvas, opts.cursor.x, opts.cursor.y, center, span);
    ctx.fillStyle = 'rgba(205,217,229,0.9)';
    ctx.font = '11px sans-serif';
    ctx.fillText(`${ll.lat.toFixed(5)}, ${ll.lon.toFixed(5)}`, 8, h - 8);
  }
}

function drawGraticule(ctx, w, h, center, span) {
  const step = niceSpan(span / 4); // 约 4 条刻度
  ctx.strokeStyle = '#15202e';
  ctx.fillStyle = 'rgba(125,138,160,0.8)';
  ctx.font = '10px sans-serif';
  ctx.lineWidth = 1;
  // 经线
  const lon0 = Math.ceil((center.lon - span) / step) * step;
  for (let lon = lon0; lon <= center.lon + span; lon += step) {
    const p = project(center.lat, lon, w, h, center, span);
    if (p.x < 0 || p.x > w) continue;
    ctx.beginPath(); ctx.moveTo(p.x, 0); ctx.lineTo(p.x, h); ctx.stroke();
    ctx.fillText(formatLon(lon), p.x + 2, 12);
  }
  // 纬线
  const lat0 = Math.ceil((center.lat - span) / step) * step;
  for (let lat = lat0; lat <= center.lat + span; lat += step) {
    const p = project(lat, center.lon, w, h, center, span);
    if (p.y < 0 || p.y > h) continue;
    ctx.beginPath(); ctx.moveTo(0, p.y); ctx.lineTo(w, p.y); ctx.stroke();
    ctx.fillText(formatLat(lat), 2, p.y - 2);
  }
}

function drawScaleBar(ctx, w, h, center, span) {
  // 取一个"好看"的米数：画布宽度的 1/4 对应的米数取整到 1-2-5
  const metersWidth = (span * 2) * M_PER_DEG; // 全宽对应的米数（span 为半宽）
  const target = metersWidth / 4;
  const pow = Math.pow(10, Math.floor(Math.log10(target)));
  const f = target / pow;
  const m = (f < 1.5 ? 1 : f < 3.5 ? 2 : f < 7.5 ? 5 : 10) * pow;
  const px = (m / M_PER_DEG) / span * (w / 2) * view.scale;
  const x0 = 12, y0 = h - 26;
  ctx.strokeStyle = '#cdd9e5';
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.moveTo(x0, y0); ctx.lineTo(x0 + px, y0);
  ctx.moveTo(x0, y0 - 4); ctx.lineTo(x0, y0 + 4);
  ctx.moveTo(x0 + px, y0 - 4); ctx.lineTo(x0 + px, y0 + 4);
  ctx.stroke();
  ctx.fillStyle = '#cdd9e5';
  ctx.font = '10px sans-serif';
  ctx.fillText(m >= 1000 ? (m / 1000) + ' km' : m + ' m', x0 + px + 6, y0 + 4);
}

function drawCompass(ctx, w, h) {
  const cx = w - 26, cy = 26, r = 14;
  ctx.strokeStyle = '#3a4658';
  ctx.fillStyle = '#cdd9e5';
  ctx.lineWidth = 1;
  ctx.beginPath(); ctx.arc(cx, cy, r, 0, Math.PI * 2); ctx.stroke();
  ctx.beginPath();
  ctx.moveTo(cx, cy - r); ctx.lineTo(cx - 3, cy); ctx.lineTo(cx + 3, cy); ctx.closePath();
  ctx.fillStyle = '#f85149'; ctx.fill();
  ctx.fillStyle = '#cdd9e5';
  ctx.font = '9px sans-serif';
  ctx.fillText('N', cx - 3, cy - r - 2);
}

function drawPlane(ctx, x, y, yawDeg, selected) {
  const yaw = (yawDeg * Math.PI) / 180;
  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(-yaw);
  ctx.fillStyle = selected ? '#2f81f7' : '#56d364';
  ctx.beginPath();
  ctx.moveTo(0, -10);
  ctx.lineTo(7, 8);
  ctx.lineTo(0, 4);
  ctx.lineTo(-7, 8);
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function formatLat(lat) {
  const h = lat >= 0 ? 'N' : 'S';
  return h + Math.abs(lat).toFixed(4) + '°';
}
function formatLon(lon) {
  const h = lon >= 0 ? 'E' : 'W';
  return h + Math.abs(lon).toFixed(4) + '°';
}

// 命中测试：返回离 (px,py) 最近的航点索引（像素阈值内），否则 -1
export function hitWaypoint(canvas, state, px, py, center, span) {
  const mission = state.mission || [];
  let best = -1, bestD = 12;
  mission.forEach((wp, i) => {
    const p = project(wp.x, wp.y, canvas.width, canvas.height, center, span);
    const d = Math.hypot(p.x - px, p.y - py);
    if (d < bestD) { bestD = d; best = i; }
  });
  return best;
}
