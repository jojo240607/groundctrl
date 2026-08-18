import serial, time, struct, sys

PORT = 'COM12'
BAUD = 115200

ctl = serial.Serial(PORT, BAUD, timeout=0.5)
ctl.dtr = False
ctl.rts = False
time.sleep(0.4)

CRC_TABLE = []
for i in range(256):
    c = i
    for _ in range(8):
        c = (c >> 1) ^ 0x8408 if (c & 1) else (c >> 1)
    CRC_TABLE.append(c)

def crc16(buf, extra):
    crc = 0xFFFF
    for x in buf:
        crc = (crc >> 8) ^ CRC_TABLE[((crc ^ x) & 0xFF)]
    crc ^= extra
    crc = (crc >> 8) ^ CRC_TABLE[(crc & 0xFF) ^ 0x00]
    crc = (crc >> 8) ^ CRC_TABLE[(crc & 0xFF)]
    return crc

def cmd_long(command, params=(0,0,0,0,0,0,0), confirmation=0, sys=1, comp=1, seq=0):
    payload = bytes([sys, comp, command & 0xFF, command >> 8, confirmation])
    payload += b''.join(struct.pack('<f', x) for x in params)
    body = bytes([0xFD, len(payload), 0,0, seq, 1,1, 76,0,0]) + payload
    c = crc16(body[1:], 152)
    ctl.write(body + bytes([c & 0xFF, c >> 8]))

def param_request_list(seq=0):
    payload = bytes([1,1])
    body = bytes([0xFD, len(payload), 0,0, seq, 1,1, 21,0,0]) + payload
    c = crc16(body[1:], 159)
    ctl.write(body + bytes([c & 0xFF, c >> 8]))

def param_set(name, value, seq=0):
    pl = name.ljust(16, '\0').encode()[:16]
    pl += struct.pack('<f', value)
    pl += bytes([1,1])  # target sys/comp
    body = bytes([0xFD, len(pl), 0,0, seq, 1,1, 23,0,0]) + pl
    c = crc16(body[1:], 168)
    ctl.write(body + bytes([c & 0xFF, c >> 8]))

def param_request_read(name, idx=-1, seq=0):
    pl = name.ljust(16, '\0').encode()[:16]
    pl += struct.pack('<i', idx)
    pl += bytes([1,1])
    body = bytes([0xFD, len(pl), 0,0, seq, 1,1, 20,0,0]) + pl
    c = crc16(body[1:], 214)
    ctl.write(body + bytes([c & 0xFF, c >> 8]))

def dump_window(dur, label):
    ctl.reset_input_buffer()
    t0 = time.time()
    seen = {}
    last_hb = None
    while time.time() - t0 < dur:
        b = ctl.read(512)
        i = 0
        while i + 9 < len(b):
            if b[i] == 0xFD:
                ln = b[i+1]; mid = b[i+7]|(b[i+8]<<8)|(b[i+9]<<16); tot = 10+ln+2
                if i+tot <= len(b):
                    seen[mid] = seen.get(mid,0)+1
                    if mid == 0:  # heartbeat
                        base = b[i+10]; custom = b[i+10+4]|(b[i+10+5]<<8)
                        last_hb = (base, custom)
                    i += tot; continue
            i += 1
    print(f'[{label}] msgid_hist={dict(sorted(seen.items()))} last_heartbeat(base,custom)={last_hb}')

MODE_NAMES = {0:'STABILIZE',2:'ALT_HOLD',5:'LOITER',6:'AUTO',9:'LAND',11:'RTL',4:'GUIDED'}

print('=== 联调: 地面站所有功能 ===')
seq = 0
# 0) 初始心跳状态
dump_window(2, 'initial')
# 1) ARM
seq += 1; cmd_long(400, (1.0,), seq=seq); time.sleep(0.3)
dump_window(3, 'after ARM')
# 2) DO_SET_MODE -> ALT_HOLD(2)
seq += 1; cmd_long(176, (1.0, 2.0), seq=seq); time.sleep(0.3)
dump_window(3, 'after DO_SET_MODE ALT_HOLD')
# 3) DO_SET_MODE -> LOITER(5)
seq += 1; cmd_long(176, (1.0, 5.0), seq=seq); time.sleep(0.3)
dump_window(3, 'after DO_SET_MODE LOITER')
# 4) TAKEOFF
seq += 1; cmd_long(22, (0,0,0,0,0,0,5.0), seq=seq); time.sleep(0.3)
dump_window(3, 'after TAKEOFF')
# 5) LAND
seq += 1; cmd_long(21, seq=seq); time.sleep(0.3)
dump_window(3, 'after LAND')
# 6) RTL
seq += 1; cmd_long(20, seq=seq); time.sleep(0.3)
dump_window(3, 'after RTL')
# 7) PARAM_SET KpXY = 1.5
seq += 1; param_set('KpXY', 1.5, seq=seq); time.sleep(0.3)
dump_window(3, 'after PARAM_SET KpXY=1.5')
# 8) PARAM_REQUEST_READ KpXY
seq += 1; param_request_read('KpXY', seq=seq); time.sleep(0.3)
dump_window(3, 'after PARAM_REQUEST_READ KpXY')
# 9) PARAM_REQUEST_LIST
seq += 1; param_request_list(seq=seq); time.sleep(0.6)
dump_window(3, 'after PARAM_REQUEST_LIST')
# 10) CAP
seq += 1; cmd_long(520, (1.0,), seq=seq); time.sleep(0.4)
dump_window(3, 'after REQUEST_AUTOPILOT_CAPABILITIES')
# 11) DISARM
seq += 1; cmd_long(400, (0.0,), seq=seq); time.sleep(0.3)
dump_window(3, 'after DISARM')
ctl.close()
print('=== done ===')
