// 仪表：空速、地速、电量、电压。用半圆/圆环表盘。

function arc(ctx, cx, cy, r, a0, a1, color, width) {
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.beginPath();
  ctx.arc(cx, cy, r, a0, a1);
  ctx.stroke();
}

function gauge(ctx, cx, cy, r, value, max, label, unit) {
  const a0 = Math.PI * 0.75, a1 = Math.PI * 2.25;
  arc(ctx, cx, cy, r, a0, a1, '#26313f', 8);
  const t = Math.max(0, Math.min(1, value / max));
  const ang = a0 + (a1 - a0) * t;
  const col = value > max * 0.85 ? '#f85149' : value > max * 0.6 ? '#d29922' : '#3fb950';
  arc(ctx, cx, cy, r, a0, ang, col, 8);
  ctx.fillStyle = '#e6edf3';
  ctx.font = 'bold 16px sans-serif';
  ctx.textAlign = 'center';
  ctx.fillText(value.toFixed(1), cx, cy + 4);
  ctx.fillStyle = '#7d8aa0';
  ctx.font = '10px sans-serif';
  ctx.fillText(unit, cx, cy + 18);
  ctx.fillText(label, cx, cy + r + 12);
  ctx.textAlign = 'left';
}

export function drawGauges(canvas, v) {
  const ctx = canvas.getContext('2d');
  const w = canvas.width, h = canvas.height;
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = '#0a0d12';
  ctx.fillRect(0, 0, w, h);

  const air = v ? v.airSpeed || 0 : 0;
  const gnd = v ? v.groundSpeed || 0 : 0;
  const batt = v ? v.battery || 0 : 0;
  const volts = v ? v.voltage || 0 : 0;

  gauge(ctx, w * 0.3, h * 0.32, 34, air, 30, '空速', 'm/s');
  gauge(ctx, w * 0.7, h * 0.32, 34, gnd, 30, '地速', 'm/s');

  // 电量条
  const bx = 24, by = h - 46, bw = w - 48, bh = 14;
  ctx.fillStyle = '#26313f';
  ctx.fillRect(bx, by, bw, bh);
  const bf = Math.max(0, Math.min(1, batt / 100));
  ctx.fillStyle = batt < 15 ? '#f85149' : batt < 30 ? '#d29922' : '#3fb950';
  ctx.fillRect(bx, by, bw * bf, bh);
  ctx.strokeStyle = '#3a4658';
  ctx.strokeRect(bx, by, bw, bh);
  ctx.fillStyle = '#e6edf3';
  ctx.font = '11px sans-serif';
  ctx.fillText(`电量 ${batt.toFixed(0)}%   电压 ${volts.toFixed(1)}V`, bx, by - 6);
}
