// 趋势图：高度(m)、地速(m/s)、电量(%) 三条曲线，按时间对齐。

export function drawTrend(canvas, tr) {
  const ctx = canvas.getContext('2d');
  const w = canvas.width, h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = '#0a0d12';
  ctx.fillRect(0, 0, w, h);

  const n = tr.t.length;
  if (n < 2) {
    ctx.fillStyle = '#7d8aa0';
    ctx.font = '12px sans-serif';
    ctx.fillText('等待数据...', 12, h / 2);
    return;
  }

  // 三条曲线各自归一化
  const series = [
    { data: tr.alt, color: '#2f81f7', max: 200, label: '高度' },
    { data: tr.spd, color: '#3fb950', max: 30, label: '地速' },
    { data: tr.batt, color: '#d29922', max: 100, label: '电量' },
  ];

  ctx.strokeStyle = '#15202e';
  for (let i = 1; i < 5; i++) {
    const y = (h / 5) * i;
    ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(w, y); ctx.stroke();
  }

  const pad = 6;
  for (const s of series) {
    ctx.strokeStyle = s.color;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const x = pad + (w - 2 * pad) * (i / (n - 1));
      const val = Math.max(0, Math.min(s.max, s.data[i] || 0));
      const y = h - pad - (h - 2 * pad) * (val / s.max);
      if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
    }
    ctx.stroke();
  }

  // 图例
  let lx = 10;
  for (const s of series) {
    ctx.fillStyle = s.color;
    ctx.fillRect(lx, 8, 10, 10);
    ctx.fillStyle = '#cdd9e5';
    ctx.font = '11px sans-serif';
    ctx.fillText(s.label, lx + 14, 17);
    lx += 70;
  }
}
