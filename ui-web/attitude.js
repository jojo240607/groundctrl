// 姿态仪：人工地平线，按 roll/pitch 旋转，中心固定，绘制偏航。

export function drawAttitude(canvas, v) {
  const ctx = canvas.getContext('2d');
  const w = canvas.width, h = canvas.height;
  const cx = w / 2, cy = h / 2, R = Math.min(w, h) / 2 - 6;
  const roll = v && v.roll ? v.roll : 0;
  const pitch = v && v.pitch ? v.pitch : 0;
  const yaw = v && v.yaw ? v.yaw : 0;

  ctx.clearRect(0, 0, w, h);

  // 裁剪圆
  ctx.save();
  ctx.beginPath();
  ctx.arc(cx, cy, R, 0, Math.PI * 2);
  ctx.clip();

  ctx.save();
  ctx.translate(cx, cy);
  ctx.rotate(-roll);
  const pp = (pitch / 90) * R; // 俯仰位移
  // 天空
  ctx.fillStyle = '#1f6feb';
  ctx.fillRect(-R * 2, -R * 2 - pp, R * 4, R * 2 + pp);
  // 地面
  ctx.fillStyle = '#8b5a2b';
  ctx.fillRect(-R * 2, pp, R * 4, R * 2);
  // 地平线
  ctx.strokeStyle = '#fff';
  ctx.lineWidth = 1.5;
  ctx.beginPath(); ctx.moveTo(-R * 2, pp); ctx.lineTo(R * 2, pp); ctx.stroke();
  // 俯仰刻度
  ctx.strokeStyle = 'rgba(255,255,255,0.6)';
  ctx.fillStyle = '#fff';
  ctx.font = '9px sans-serif';
  for (let d = -30; d <= 30; d += 10) {
    if (d === 0) continue;
    const yy = pp - (d / 90) * R;
    ctx.beginPath(); ctx.moveTo(-20, yy); ctx.lineTo(20, yy); ctx.stroke();
  }
  ctx.restore();

  // 滚转刻度弧
  ctx.strokeStyle = 'rgba(255,255,255,0.5)';
  ctx.beginPath(); ctx.arc(cx, cy, R - 4, Math.PI, 2 * Math.PI); ctx.stroke();
  ctx.restore();

  // 固定飞机符号
  ctx.strokeStyle = '#ffcc00';
  ctx.lineWidth = 3;
  ctx.beginPath();
  ctx.moveTo(cx - 28, cy); ctx.lineTo(cx - 10, cy);
  ctx.moveTo(cx + 10, cy); ctx.lineTo(cx + 28, cy);
  ctx.moveTo(cx, cy - 4); ctx.lineTo(cx, cy + 4);
  ctx.stroke();
  ctx.fillStyle = '#ffcc00';
  ctx.beginPath(); ctx.arc(cx, cy, 3, 0, Math.PI * 2); ctx.fill();

  // 外圈 + 读数
  ctx.strokeStyle = '#3a4658';
  ctx.beginPath(); ctx.arc(cx, cy, R, 0, Math.PI * 2); ctx.stroke();
  ctx.fillStyle = '#cdd9e5';
  ctx.font = '11px sans-serif';
  ctx.textAlign = 'center';
  ctx.fillText(`R:${(roll * 180 / Math.PI).toFixed(0)}° P:${(pitch * 180 / Math.PI).toFixed(0)}°`, cx, h - 6);
  ctx.fillText(`YAW:${((yaw * 180 / Math.PI) % 360).toFixed(0)}°`, cx, 14);
  ctx.textAlign = 'left';
}
