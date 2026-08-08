// 地图：基于 Leaflet 的真实开源瓦片底图（OpenStreetMap，无需 API key）。
// 航点/飞机/围栏用 Leaflet 图层叠加在真实 WGS84 坐标系上——和数据天然对齐、
// 可缩放、可拖拽，和主流地面站一致。瓦片不可达时降级提示，航点编辑仍可用。
import L from 'leaflet';

let map = null;
let wpLayer = null;       // 航点图层组
let vehMarkers = new Map(); // sysid -> marker
let fenceLayer = null;
let offline = false;

// 初始化 Leaflet 地图（只调用一次）
export function initMap(divEl, opts = {}) {
  map = L.map(divEl, { preferCanvas: true, zoomControl: true, contextmenu: true })
    .setView([31.0, 121.0], 13);

  const tiles = L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png', {
    maxZoom: 19,
    attribution: '&copy; OpenStreetMap contributors',
  });
  tiles.on('tileerror', () => {
    if (!offline) {
      offline = true;
      showOfflineBadge(true);
      opts.onOffline && opts.onOffline(true);
    }
  });
  tiles.on('tileload', () => {
    if (offline) {
      offline = false;
      showOfflineBadge(false);
      opts.onOffline && opts.onOffline(false);
    }
  });
  tiles.addTo(map);

  // 尝试用浏览器定位到操作员当前位置（需授权；失败则保持默认视图，不阻塞）
  if (typeof navigator !== 'undefined' && navigator.geolocation) {
    navigator.geolocation.getCurrentPosition(
      (pos) => {
        if (map) map.setView([pos.coords.latitude, pos.coords.longitude], 15);
        opts.onOperatorLocated && opts.onOperatorLocated(pos.coords.latitude, pos.coords.longitude);
      },
      () => { /* 拒绝/不可用：保持默认视图 */ },
      { enableHighAccuracy: false, timeout: 8000, maximumAge: 600000 }
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
}

export function isOffline() { return offline; }

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

// 同步围栏
export function renderFence(fence) {
  if (!map) return;
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
