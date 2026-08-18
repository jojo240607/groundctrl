#!/usr/bin/env python3
# 地面站 WebUI 无头浏览器联调桥。
#
# 作用：
#   在无 Tauri 运行时的纯浏览器环境下，用 WebSocket 把 WebUI 的 invoke/listen
#   语义桥接到真实串口 (COM12, 板子 USB CDC)。复用已验证的标准 MAVLink v2 编解码。
#
# 运行：
#   python gc_bridge.py [PORT] [BAUD]
# 默认 PORT=COM12 BAUD=115200。WebSocket 监听 ws://localhost:8787。
#
# WebUI 侧已在 main.js 加入浏览器 fallback：当 window.__TAURI__ 不存在时，
# 自动经 ws://localhost:8787 走本桥。

import asyncio, json, struct, sys, time, math

try:
    import serial
except ImportError:
    serial = None

try:
    import serial.tools.list_ports
except Exception:
    serial = None

import websockets

PORT = sys.argv[1] if len(sys.argv) > 1 else 'COM12'
BAUD = int(sys.argv[2]) if len(sys.argv) > 2 else 115200

WS_PORT = 8787

# ---- CRC ----
# 标准 MAVLink X25/CRC-16-CCITT 反射算法，与飞控 flyctrl_core::comm::mavlink::crc16_x25 严格一致。
def crc16(buf, extra):
    # 与飞控 flyctrl_core::comm::mavlink::crc16_x25 严格一致：
    # 初值 0xFFFF，对 body（v2 头部 9 字节 + payload）逐字节反射累积，
    # 最后把 CRC_EXTRA 作为「最后一个字节」走完整 8 轮移位（不是简单异或后返回）。
    crc = 0xFFFF
    for x in buf:
        crc ^= x
        for _ in range(8):
            crc = ((crc >> 1) ^ 0x8408) if (crc & 1) else (crc >> 1)
    crc ^= extra
    for _ in range(8):
        crc = ((crc >> 1) ^ 0x8408) if (crc & 1) else (crc >> 1)
    return crc & 0xFFFF

SYS_ID = 1
COMP_ID = 1

def enc_command_long(command, params=(0,0,0,0,0,0,0), confirmation=0, seq=0):
    pl = bytes([SYS_ID, COMP_ID, command & 0xFF, command >> 8, confirmation])
    pl += b''.join(struct.pack('<f', float(x)) for x in params)
    body = bytes([0xFD, len(pl), 0, 0, seq, SYS_ID, COMP_ID, 76,0,0]) + pl
    c = crc16(body[1:], 152)
    return body + bytes([c & 0xFF, c >> 8])

def enc_param_request_list(seq=0):
    pl = bytes([SYS_ID, COMP_ID])
    body = bytes([0xFD, len(pl), 0,0, seq, SYS_ID, COMP_ID, 21,0,0]) + pl
    c = crc16(body[1:], 159)
    return body + bytes([c & 0xFF, c >> 8])

def enc_param_set(name, value, seq=0):
    n = name.encode()[:16].ljust(16, b'\x00')
    pl = n + struct.pack('<f', float(value)) + bytes([SYS_ID, COMP_ID])
    body = bytes([0xFD, len(pl), 0,0, seq, SYS_ID, COMP_ID, 23,0,0]) + pl
    c = crc16(body[1:], 168)
    return body + bytes([c & 0xFF, c >> 8])

def enc_param_request_read(name, idx=-1, seq=0):
    n = name.encode()[:16].ljust(16, b'\x00')
    pl = n + struct.pack('<i', idx) + bytes([SYS_ID, COMP_ID])
    body = bytes([0xFD, len(pl), 0,0, seq, SYS_ID, COMP_ID, 20,0,0]) + pl
    c = crc16(body[1:], 214)
    return body + bytes([c & 0xFF, c >> 8])

MODE_NAMES = {0:'STABILIZE',1:'ACRO',2:'ALT_HOLD',3:'POSHOLD',4:'GUIDED',5:'LOITER',
              6:'AUTO',7:'CIRCLE',9:'LAND',11:'RTL',12:'DRIFT',13:'SPORT',15:'GUIDED_NOGPS',16:'NORMAL'}

# MAVLink v2 CRC_EXTRA（common.xml），用于入站帧校验与帧同步恢复。
# 下行已覆盖 HB(0)/SYS_STATUS(1)/ATTITUDE(30)/LOCAL_POSITION_NED(32)/GLOBAL_POSITION_INT(33)/VFR_HUD(74)；
# 上行指令需覆盖 COMMAND_LONG(76)/COMMAND_ACK(77)，否则发出的 ARM/DISARM 等指令 CRC 错误
# 被飞控拒绝（decode FAILED），界面无法同步解锁状态。
# 参数流：PARAM_REQUEST_READ(20)/PARAM_REQUEST_LIST(21)/PARAM_VALUE(22)/PARAM_SET(23) ——
# 缺这几项会导致板子下发的 PARAM_VALUE 入站 CRC 校验失败（extra 缺省 0）被当成错位帧丢弃，
# 界面拉取参数永远为 0 条。常量与下方 enc_* 出站编码保持一致（与飞控 mavlink.rs 对齐）。
CRC_EXTRA = {0:50, 1:124, 20:214, 21:159, 22:220, 23:168, 30:39, 32:185, 33:104, 74:20, 76:152, 77:143}

# ---- 当前遥测状态 ----
class Snap:
    def __init__(self):
        self.online = False
        self.armed = None
        self.base_mode = 0
        self.custom_mode = 0
        self.flight_mode = '--'
        self.battery = None
        self.voltage = None
        self.current = None
        self.lat = None
        self.lon = None
        self.alt_rel = None
        self.alt_abs = None
        self.roll = None
        self.pitch = None
        self.yaw = None
        self.ground_speed = None
        self.air_speed = None
        self.heading = None
        self.gps_fix = None
        self.satellites = None
        self.last_seen = 0

snap = Snap()

def snapshot_json():
    return _clean({
        'vehicles': [{
            'sysid': SYS_ID, 'compid': COMP_ID, 'name': 'usb0', 'connected': snap.online,
            'flightMode': snap.flight_mode, 'armed': snap.armed,
            'battery': snap.battery, 'voltage': snap.voltage, 'current': snap.current,
            'lat': snap.lat, 'lon': snap.lon, 'altRel': snap.alt_rel, 'altAbs': snap.alt_abs,
            'relativeAlt': snap.alt_rel,
            'vx': None, 'vy': None, 'vz': None,
            'roll': snap.roll, 'pitch': snap.pitch, 'yaw': snap.yaw,
            'groundSpeed': snap.ground_speed, 'airSpeed': snap.air_speed, 'heading': snap.heading,
            'gpsFix': snap.gps_fix, 'satellites': snap.satellites, 'hdop': None,
            'lastUpdate': int(snap.last_seen),
        }],
        'selected': SYS_ID,
    })

def _clean(v):
    """把 NaN/inf/None 统一成 None，避免 json.dumps 因 NaN 输出字面 NaN（非法 JSON），
    导致前端 JSON.parse 崩溃、fleet 事件整体被丢弃、界面永远不同步。"""
    if v is None:
        return None
    if isinstance(v, float) and (math.isnan(v) or math.isinf(v)):
        return None
    if isinstance(v, dict):
        return {k: _clean(x) for k, x in v.items()}
    if isinstance(v, list):
        return [_clean(x) for x in v]
    return v

# ---- 下行帧解析 ----
def parse_downlink(buf, seq_state):
    """buf: 从串口读到的原始字节。返回 (events, remaining) ，remaining 是尚未解析完整的尾部字节。"""
    events = []
    b = buf
    i = 0
    n = len(b)
    while i + 9 < n:
        if b[i] == 0xFD:
            ln = b[i+1]
            mid = b[i+7] | (b[i+8] << 8) | (b[i+9] << 16)
            tot = 10 + ln + 2
            if i + tot > n:
                break  # 帧不完整，留在 remaining
            # CRC 校验：防止 payload 内含 0xFD 导致的帧错位污染快照。
            # 校验失败说明本字节不是真帧头，跳 1 字节继续扫描（帧同步恢复）。
            extra = CRC_EXTRA.get(mid, 0)
            body = b[i+1:i+tot-2]
            crc_recv = b[i+tot-2] | (b[i+tot-1] << 8)
            if crc16(body, extra) != crc_recv:
                i += 1
                continue
            pl = b[i+10:i+10+ln]
            if mid == 0:  # HEARTBEAT
                base_mode = pl[2]
                custom_mode = struct.unpack('<I', pl[3:7])[0]
                snap.online = True
                snap.base_mode = base_mode
                snap.custom_mode = custom_mode
                snap.armed = bool(base_mode & 0x80)
                snap.flight_mode = MODE_NAMES.get(custom_mode, 'MODE%d' % custom_mode)
                snap.last_seen = time.time()
            elif mid == 30:  # ATTITUDE
                roll, pitch, yaw = struct.unpack('<fff', pl[4:16])
                snap.roll, snap.pitch, snap.yaw = roll, pitch, yaw
            elif mid == 74:  # VFR_HUD
                airspeed, groundspeed, heading = struct.unpack('<fff', pl[0:12])
                snap.air_speed = airspeed
                snap.ground_speed = groundspeed
                snap.heading = heading
            elif mid == 33:  # GLOBAL_POSITION_INT
                lat, lon, alt, rel = struct.unpack('<iiii', pl[4:20])
                snap.lat = lat / 1e7
                snap.lon = lon / 1e7
                snap.alt_abs = alt / 1000.0
                snap.alt_rel = rel / 1000.0
            elif mid == 1:  # SYS_STATUS
                volt, cur, rem = struct.unpack('<HhB', pl[12:17])
                snap.voltage = volt / 1000.0
                snap.current = cur / 100.0
                snap.battery = float(rem)
            elif mid == 32:  # LOCAL_POSITION_NED
                pass  # 可选
            elif mid == 22:  # PARAM_VALUE
                name = pl[:16].split(b'\x00')[0].decode('ascii', 'replace')
                value = struct.unpack('<f', pl[16:20])[0]
                idx, count = struct.unpack('<HH', pl[20:24])
                events.append(('param-value', {'index': idx, 'name': name, 'value': value,
                                               'received': idx + 1, 'expected': count}))
            i += tot
            continue
        i += 1
    return events, b[i:]

# ---- WebSocket 桥 ----
CLIENTS = set()
serial_port = None
seq = 0

async def ws_handler(ws):
    CLIENTS.add(ws)
    try:
        async for raw in ws:
            try:
                msg = json.loads(raw)
            except Exception:
                continue
            if msg.get('type') != 'invoke':
                continue
            cmd = msg.get('cmd')
            args = msg.get('args', {}) or {}
            mid = msg.get('id')
            try:
                result = await handle_invoke(cmd, args)
                await ws.send(json.dumps({'type': 'resp', 'id': mid, 'ok': True, 'data': result}, allow_nan=False))
            except Exception as e:
                await ws.send(json.dumps({'type': 'resp', 'id': mid, 'ok': False, 'error': str(e)}, allow_nan=False))
    finally:
        CLIENTS.discard(ws)

async def handle_invoke(cmd, args):
    global serial_port, seq
    if cmd == 'list_serial_ports':
        ports = []
        try:
            for p in serial.tools.list_ports.comports():
                ports.append(p.device)
        except Exception:
            pass
        return ports if ports else [PORT]
    if cmd == 'get_settings':
        return {'default_url': 'serial:%s' % PORT, 'trend_enabled': False, 'trend_selected': []}
    if cmd == 'get_monitor_config':
        return {'batteryWarnPct': 30, 'batteryCriticalPct': 15, 'fenceRadiusM': 0.0,
                'fenceLat': 0.0, 'fenceLon': 0.0}
    if cmd == 'set_monitor_config':
        return None
    if cmd == 'connect':
        if serial_port is None or not serial_port.is_open:
            serial_port = serial.Serial(PORT, BAUD, timeout=0.2)
            serial_port.dtr = False
            serial_port.rts = False
            await broadcast_event('link-state', {'link': 'usb0', 'connected': True})
        snap.online = True
        return None
    if cmd == 'disconnect':
        if serial_port and serial_port.is_open:
            serial_port.close()
        snap.online = False
        await broadcast_event('link-state', {'link': 'usb0', 'connected': False})
        return None
    if cmd == 'get_fleet':
        return snapshot_json()
    if cmd == 'request_params':
        seq = (seq + 1) & 0xFF
        serial_port.write(enc_param_request_list(seq))
        await broadcast_event('params-progress', {'received': 0, 'expected': 0})
        return None
    if cmd == 'set_param':
        name = args.get('name', '')
        value = float(args.get('value', 0))
        seq = (seq + 1) & 0xFF
        serial_port.write(enc_param_set(name, value, seq))
        return None
    if cmd == 'upload_mission':
        return None
    if cmd == 'download_mission':
        return []
    if cmd == '__raw':
        # 直接发送调用方构造的原始 MAVLink 帧（用于 UI 未暴露的指令闭环测试）
        frame = bytes(args.get('frame', []))
        if frame and serial_port and serial_port.is_open:
            serial_port.write(frame)
        return None
    return None

async def broadcast_event(event, payload):
    dead = []
    for ws in CLIENTS:
        try:
            await ws.send(json.dumps({'type': 'event', 'event': event, 'payload': payload}, allow_nan=False))
        except Exception:
            dead.append(ws)
    for ws in dead:
        CLIENTS.discard(ws)

async def serial_reader():
    global serial_port
    buf = b''
    last_fleet = 0
    while True:
        await asyncio.sleep(0.01)
        if serial_port is None or not serial_port.is_open:
            await asyncio.sleep(0.1)
            continue
        try:
            chunk = serial_port.read(512)
        except Exception:
            chunk = b''
        if not chunk:
            continue
        buf += chunk
        if len(buf) > 8192:
            buf = buf[-4096:]
        events, buf = parse_downlink(buf, None)
        for et, pl in events:
            if et == 'param-value':
                await broadcast_event('param-value', pl)
                if pl.get('expected') and pl['received'] < pl['expected']:
                    await broadcast_event('params-progress',
                        {'received': pl['received'], 'expected': pl['expected']})
        now = time.time()
        if now - last_fleet > 0.2:
            last_fleet = now
            await broadcast_event('fleet', snapshot_json())

async def main():
    if serial is None:
        print('pyserial 不可用，无法桥接串口')
        return
    async with websockets.serve(ws_handler, 'localhost', WS_PORT):
        print('gc_bridge: WebSocket ws://localhost:8787  串口 %s @ %d' % (PORT, BAUD))
        await serial_reader()

if __name__ == '__main__':
    asyncio.run(main())
