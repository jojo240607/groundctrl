// 遥测趋势图：多通道可切换折线图，带 Y 轴数值刻度、时间轴与当前值标注。
// 通道由 state.trend 提供，可见通道由 channels 数组控制（main.js 维护）。

const CHANNELS = [
  { key: 'alt',  color: '#2f81f7', max: 200, unit: 'm',   label: '高度' },
  { key: 'spd',  color: '#3fb950', max: 30,  unit: 'm/s', label: '地速' },
  { key: 'batt', color: '#d29922', max: 100, unit: '%',   label: '电量' },
  { key: 'air',  color: '#a371f7', max: 30,  unit: 'm/s', label: '空速' },
];

export function trendChannels() { return CHANNELS; }

export function drawTrend(canvas, tr, visible) {
  const ctx = canvas.getContext('2d');
  const w = canvas.width, h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = '#0a0d12';
  ctx.fillRect(0, 0, w, h);

  const padL = 38, padR = 8, padT = 8, padB = 16;
  const plotW = w - padL - padR, plotH = h - padT - padB;
  const n = tr.t.length;

  if (n < 2) {
    ctx.fillStyle = '#7d8aa0';
    ctx.font = '12px sans-serif';
    ctx.fillText('等待数据...', 12, h / 2);
    return;
  }

  const visCh = CHANNELS.filter(c => !visible || visible.includes(c.key));
  if (visCh.length === 0) return;

  // 网格 + Y 轴刻度（按各通道自适应上限，取可见通道最大 max 作为统一参考栅格）
  ctx.strokeStyle = '#15202e';
  ctx.lineWidth = 1;
  const gridN = 4;
  ctx.fillStyle = '#5b6a7e';
  ctx.font = '9px sans-serif';
  for (let i = 0; i <= gridN; i++) {
    const y = padT + (plotH * i) / gridN;
    ctx.beginPath(); ctx.moveTo(padL, y); ctx.lineTo(w - padR, y); ctx.stroke();
  }

  const maxT = tr.t[n - 1], minT = tr.t[0];
  const spanT = Math.max(1, maxT - minT);

  for (const c of visCh) {
    ctx.strokeStyle = c.color;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const x = padL + plotW * ((tr.t[i] - minT) / spanT);
      const val = Math.max(0, Math.min(c.max, tr[c.key] ? tr[c.key][i] || 0 : 0));
      const y = padT + plotH - plotH * (val / c.max);
      if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
    }
    ctx.stroke();
    // 当前值
    const last = tr[c.key] ? (tr[c.key][n - 1] || 0) : 0;
    ctx.fillStyle = c.color;
    ctx.font = '10px sans-serif';
    ctx.fillText(`${last.toFixed(1)}${c.unit}`, padL + 2, padT + 10 + visCh.indexOf(c) * 12);
  }

  // X 轴时间标注
  ctx.fillStyle = '#5b6a7e';
  ctx.font = '9px sans-serif';
  const dt = ((spanT / 1000) | 0);
  ctx.fillText(`-${dt}s`, padL, h - 4);
  ctx.fillText('now', w - padR - 18, h - 4);
}
