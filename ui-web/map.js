// 地图：基于 Leaflet 的真实开源瓦片底图（OpenStreetMap，无需 API key）。
// 航点/飞机/围栏用 Leaflet 图层叠加在真实 WGS84 坐标系上——和数据天然对齐、
// 可缩放、可拖拽，和主流地面站一致。瓦片不可达时降级提示，航点编辑仍可用。
import L from 'leaflet';

let map = null;
let wpLayer = null;       // 航点图层组
let vehMarkers = new Map(); // sysid -> marker
let fenceLayer = null;
let opMarker = null;      // 操作员定位标记
let offline = false;
let gridLayer = null;     // 离线时绘制的经纬网
let tileLayer = null;     // 瓦片图层（供定位后强制重绘）

// 初始化 Leaflet 地图（只调用一次）
export function initMap(divEl, opts = {}) {
  map = L.map(divEl, { preferCanvas: true, zoomControl: true, contextmenu: true })
    .setView([31.0, 121.0], 13);

  let tilesLoaded = false;
  let tileLoadCount = 0, tileErrCount = 0;
  const tiles = L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png', {
    maxZoom: 19,
    attribution: '&copy; OpenStreetMap contributors',
  });
  tileLayer = tiles;
  tiles.on('tileerror', () => {
    tileErrCount++;
    console.warn('[map] tileerror #%d (offline=%s)', tileErrCount, offline);
    if (!offline) {
      offline = true;
      showOfflineBadge(true);
      opts.onOffline && opts.onOffline(true);
    }
  });
  tiles.on('tileload', () => {
    tilesLoaded = true;
    tileLoadCount++;
    if (offline) {
      offline = false;
      showOfflineBadge(false);
      clearOfflineGraticule();
      opts.onOffline && opts.onOffline(false);
    }
  });
  tiles.addTo(map);

  // 瓦片加载探测：若 6s 内没有任何瓦片成功加载（代理返回错误页/网络被拦时
  // tileerror 可能不触发），强制进入离线提示，避免一直黑屏误以为程序坏了。
  setTimeout(() => {
    console.log('[map] 6s probe: tilesLoaded=%s load=%d err=%d offline=%s',
      tilesLoaded, tileLoadCount, tileErrCount, offline);
    if (!tilesLoaded && !offline) {
      offline = true;
      showOfflineBadge(true);
      drawOfflineGraticule();
      opts.onOffline && opts.onOffline(true);
    }
  }, 6000);

  // 尝试用浏览器定位到操作员当前位置（需授权；失败则保持默认视图，不阻塞）
  if (typeof navigator !== 'undefined' && navigator.geolocation) {
    navigator.geolocation.getCurrentPosition(
      (pos) => {
        const lat = pos.coords.latitude, lng = pos.coords.longitude;
        // 诊断：把定位结果打到 console（WebView2 中按 F12 查看）
        console.log('[map] geolocation => lat=%s lng=%s', lat, lng);
        // 坐标合法性校验：非法/越界坐标会导致 Leaflet 瓦片 URL 异常、地图变黑
        const ok = typeof lat === 'number' && typeof lng === 'number'
          && isFinite(lat) && isFinite(lng)
          && lat >= -90 && lat <= 90 && lng >= -180 && lng <= 180;
        if (!ok) {
          console.warn('[map] 操作员定位坐标非法，放弃 setView，保留当前视图');
          return;
        }
        // 拒绝把地图飞到 (0,0) 这种“定位成功但无真实位置”的默认值
        if (lat === 0 && lng === 0) {
          console.warn('[map] 定位返回 (0,0) 默认值，视为无效，保留当前视图');
          return;
        }
        renderOperator(lat, lng);
        if (map) {
          console.log('[map] setView before: center=%s zoom=%d size=%s',
            map.getCenter(), map.getZoom(), JSON.stringify(map.getSize()));
          // 仅 setView：Leaflet 会基于地图当前已计算的容器尺寸请求新视口瓦片。
          // 注意：不要用 invalidateSize() —— 在异步定位回调里调用它会把容器尺寸
          // 重算为 0（WebView 布局时序），导致瓦片层变成 0 尺寸、永远不请求瓦片，
          // 地图只剩底色（这正是“定位后变黑/变蓝”的根因）。旧版没有定位时地图正常，
          // 正是因为从未调用过 invalidateSize。
          map.setView([lat, lng], 15);
          console.log('[map] setView after: center=%s zoom=%d offline=%s',
            map.getCenter(), map.getZoom(), offline);
        }
        // 离线时定位到新视口后，立即把经纬网重绘到该区域，
        // 避免“黑底 + 一个蓝点”看起来像地图坏了。
        if (offline) { clearOfflineGraticule(); drawOfflineGraticule(); }
        opts.onOperatorLocated && opts.onOperatorLocated(lat, lng);
      },
      () => { /* 拒绝/不可用：保持默认视图 */ },
      { enableHighAccuracy: false, timeout: 8000, maximumAge: 0 }
    );
  }

  wpLayer = L.layerGroup().addTo(map);

  // 点击空白：新增航点
  map.on('click', (e) => {
    opts.onWaypointAdd && opts.onWaypointAdd(e.latlng.lat, e.latlng.lng);
  });
  // 右键空白：删除最近航点
  map.on('contextmenu', (e) => {
    opts.onWaypointDeleteAt && opts.onWaypointDeleteAt(e.latlng.lat, e.latlng.lng);
  });
  // 鼠标移动：坐标读数
  map.on('mousemove', (e) => {
    opts.onCursor && opts.onCursor(e.latlng.lat, e.latlng.lng);
  });

  return map;
}

function showOfflineBadge(on) {
  let el = document.getElementById('map-offline');
  if (!el) {
    el = document.createElement('div');
    el.id = 'map-offline';
    el.className = 'map-offline';
    el.textContent = '离线：地图瓦片不可用（航点编辑仍可正常进行）';
    document.querySelector('.map-card')?.appendChild(el);
  }
  el.style.display = on ? 'block' : 'none';
  if (on) drawOfflineGraticule(); else clearOfflineGraticule();
}

export function isOffline() { return offline; }

// 离线时在地图上绘制淡色经纬网，使黑底区域明显是一张“地图画布”而非故障，
// 航点/飞机/操作员标记叠加其上仍可读。仅作视觉提示，不影响任何编辑功能。
function drawOfflineGraticule() {
  if (!map || gridLayer) return;
  const b = map.getBounds();
  const lat0 = Math.floor(b.getSouth() / 5) * 5;
  const lat1 = Math.ceil(b.getNorth() / 5) * 5;
  const lng0 = Math.floor(b.getWest() / 5) * 5;
  const lng1 = Math.ceil(b.getEast() / 5) * 5;
  const lines = [];
  for (let lat = lat0; lat <= lat1; lat += 5) lines.push([[[lat, lng0], [lat, lng1]]]);
  for (let lng = lng0; lng <= lng1; lng += 5) lines.push([[[lat0, lng], [lat1, lng]]]);
  gridLayer = L.layerGroup(lines.map((seg) =>
    L.polyline(seg[0], { color: '#3a4658', weight: 1, opacity: 0.9, interactive: false })
  )).addTo(map);
  const redraw = () => { if (offline) { clearOfflineGraticule(); drawOfflineGraticule(); } };
  map.on('moveend', redraw);
  gridLayer._redraw = redraw;
}
function clearOfflineGraticule() {
  if (gridLayer) {
    if (gridLayer._redraw) map.off('moveend', gridLayer._redraw);
    map.removeLayer(gridLayer);
    gridLayer = null;
  }
}

export function panTo(lat, lng) {
  if (map) map.setView([lat, lng], Math.max(map.getZoom(), 15));
}

// 同步航点图层
export function renderWaypoints(mission, selectedWp, opts = {}) {
  if (!wpLayer) return;
  wpLayer.clearLayers();
  mission.forEach((wp, i) => {
    const m = L.circleMarker([wp.x, wp.y], {
      radius: i === selectedWp ? 8 : 6,
      color: i === selectedWp ? '#f0a500' : '#3fb950',
      fillColor: i === selectedWp ? '#f0a500' : '#3fb950',
      fillOpacity: 0.9,
      weight: 2,
      draggable: true,
    });
    m.bindTooltip(String(i), { permanent: false, direction: 'right' });
    m.on('click', (e) => { L.DomEvent.stop(e); opts.onSelect && opts.onSelect(i); });
    m.on('drag', (e) => {
      const ll = e.target.getLatLng();
      opts.onWaypointMove && opts.onWaypointMove(i, ll.lat, ll.lng);
    });
    m.on('dragend', (e) => {
      const ll = e.target.getLatLng();
      opts.onWaypointMove && opts.onWaypointMove(i, ll.lat, ll.lng);
      opts.onWaypointMoved && opts.onWaypointMoved(i);
    });
    m.on('contextmenu', (e) => {
      L.DomEvent.stop(e);
      opts.onWaypointDelete && opts.onWaypointDelete(i);
    });
    m.addTo(wpLayer);
  });
}

// 同步飞机位置
export function renderVehicles(vehicles, selectedSys) {
  if (!map) return;
  for (const v of vehicles) {
    if (!v.lat) continue;
    let m = vehMarkers.get(v.sysid);
    if (!m) {
      m = L.marker([v.lat, v.lon], { icon: planeIcon(v.yaw || 0, v.sysid === selectedSys) });
      m.addTo(map);
      vehMarkers.set(v.sysid, m);
    } else {
      m.setLatLng([v.lat, v.lon]);
      m.setIcon(planeIcon(v.yaw || 0, v.sysid === selectedSys));
    }
    const label = `${v.sysid} ${v.flightMode || ''}`;
    m.bindTooltip(label, { direction: 'top' });
  }
}

// 标注操作员当前位置（浏览器定位）。lat/lng 为空则清除标记。
// 注意：必须用 L.marker + divIcon（DOM 元素），不能用 L.circleMarker——
// 在 preferCanvas:true 下 circleMarker 渲染到 overlay <canvas>，该 canvas 覆盖
// 全视口且位于瓦片层之上，会盖住地图导致“定位后不显示瓦片”。divIcon 是轻量
// DOM 节点，和飞机图标同机制，不会遮挡瓦片层。
export function renderOperator(lat, lng) {
  if (!map) return;
  if (opMarker) { map.removeLayer(opMarker); opMarker = null; }
  if (lat == null || lng == null) return;
  const dot = `<svg width="18" height="18" viewBox="-9 -9 18 18">
    <circle cx="0" cy="0" r="6" fill="#2f81f7" stroke="#fff" stroke-width="2"/>
  </svg>`;
  opMarker = L.marker([lat, lng], {
    icon: L.divIcon({ html: dot, className: 'op-icon', iconSize: [18, 18], iconAnchor: [9, 9] }),
  }).addTo(map);
  opMarker.bindTooltip('操作员位置', { direction: 'top' });
}

// 同步围栏
export function renderFence(fence) {  if (!map) return;
  if (fenceLayer) { map.removeLayer(fenceLayer); fenceLayer = null; }
  if (fence) {
    fenceLayer = L.circle([fence.lat, fence.lon], {
      radius: fence.radius,
      color: '#d29922',
      weight: 1.5,
      dashArray: '5,4',
      fillOpacity: 0.05,
    }).addTo(map);
  }
}

function planeIcon(yawDeg, selected) {
  const color = selected ? '#2f81f7' : '#56d364';
  const arrow = `<svg width="24" height="24" viewBox="-12 -12 24 24" style="transform:rotate(${-yawDeg}deg)">
    <polygon points="0,-10 7,8 0,4 -7,8" fill="${color}" stroke="#0a0d12" stroke-width="1"/>
  </svg>`;
  return L.divIcon({
    html: arrow,
    className: 'plane-icon',
    iconSize: [24, 24],
    iconAnchor: [12, 12],
  });
}

// 屏幕坐标 -> 经纬度（供需要时用）
export function screenToLatLon(px, py) {
  if (!map) return null;
  const pt = map.containerPointToLatLng([px, py]);
  return { lat: pt.lat, lon: pt.lng };
}
