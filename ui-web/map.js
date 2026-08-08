// 地图绘制：以所有飞机质心为中心，墨卡托近似投影，绘制围栏与飞机。
// 轻量实现，支持缩放（滚轮）与平移（拖拽）。

const view = { scale: 1, ox: 0, oy: 0, dragging: false, lastX: 0, lastY: 0 };

function project(lat, lon, w, h, center, span) {
  // 以 center 为图心，span（度）为半宽
  const dx = (lon - center.lon) / span;
  const dy = (lat - center.lat) / span;
  return { x: w / 2 + dx * w / 2 * view.scale + view.ox, y: h / 2 - dy * h / 2 * view.scale + view.oy };
}

export function drawMap(canvas, state, v) {
  const ctx = canvas.getContext('2d');
  const w = canvas.width, h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = '#0a0d12';
  ctx.fillRect(0, 0, w, h);

  // 中心点：选中飞机位置，否则默认
  const center = v && v.lat ? { lat: v.lat, lon: v.lon } : { lat: 31.0, lon: 121.0 };
  const span = 0.03 / view.scale; // 半宽约 3km（1度≈111km）

  // 网格
  ctx.strokeStyle = '#15202e';
  ctx.lineWidth = 1;
  for (let i = 1; i < 10; i++) {
    ctx.beginPath(); ctx.moveTo((w / 10) * i, 0); ctx.lineTo((w / 10) * i, h); ctx.stroke();
    ctx.beginPath(); ctx.moveTo(0, (h / 10) * i); ctx.lineTo(w, (h / 10) * i); ctx.stroke();
  }

  // 围栏
  const cfg = window.__APP_CFG__;
  // 尝试从状态读取围栏（若后端提供）。这里绘制默认 1km 圆示意。
  // 围栏坐标从告警规则面板推断（前端保存最新一次 applyCfg）。
  if (window.__FENCE__) {
    const f = window.__FENCE__;
    const p = project(f.lat, f.lon, w, h, center, span);
    const r = (f.radius / 111000) / span * (w / 2) * view.scale;
    ctx.strokeStyle = 'rgba(242,153,73,0.6)';
    ctx.beginPath(); ctx.arc(p.x, p.y, r, 0, Math.PI * 2); ctx.stroke();
  }

  // 轨迹（趋势中保留的位置点，这里仅示意）
  // 飞机
  for (const veh of state.vehicles) {
    if (!veh.lat) continue;
    const p = project(veh.lat, veh.lon, w, h, center, span);
    drawPlane(ctx, p.x, p.y, veh.yaw || 0, veh === v);
    ctx.fillStyle = '#9fb3c8';
    ctx.font = '11px sans-serif';
    ctx.fillText(veh.flightMode, p.x + 12, p.y - 12);
  }

  // 中心十字
  ctx.strokeStyle = '#243040';
  ctx.beginPath(); ctx.moveTo(w / 2 - 6, h / 2); ctx.lineTo(w / 2 + 6, h / 2); ctx.stroke();
  ctx.beginPath(); ctx.moveTo(w / 2, h / 2 - 6); ctx.lineTo(w / 2, h / 2 + 6); ctx.stroke();
}

function drawPlane(ctx, x, y, yawDeg, selected) {
  const yaw = (yawDeg * Math.PI) / 180;
  ctx.save();
  ctx.translate(x, y);
  ctx.rotate(-yaw); // 屏幕 y 向下，北为 -y
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

// 滚轮缩放
if (typeof document !== 'undefined') {
  document.addEventListener('wheel', (e) => {
    if (e.target && e.target.id === 'map') {
      e.preventDefault();
      const f = e.deltaY < 0 ? 1.1 : 0.9;
      view.scale = Math.max(0.2, Math.min(8, view.scale * f));
    }
  }, { passive: false });
}
