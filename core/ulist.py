import serial, time, struct, sys
from collections import Counter

ctl = serial.Serial('COM12', 115200, timeout=1)
ctl.dtr = False
ctl.rts = False
time.sleep(0.3)

def crc16(buf, extra):
    crc = 0xFFFF
    for x in buf:
        crc ^= x
        for _ in range(8):
            crc = (crc >> 1) ^ 0x8408 if (crc & 1) else (crc >> 1)
    crc ^= extra
    for _ in range(8):
        crc = (crc >> 1) ^ 0x8408 if (crc & 1) else (crc >> 1)
    return crc

def send_cl(command, p1=0.0, extra=152):
    payload = bytes([1,1, command&0xff, command>>8, 0]) + b''.join(struct.pack('<f', x) for x in [p1,0,0,0,0,0,0]) + bytes([0])
    body = bytes([0xFD, len(payload), 0,0,0, 1,1, 76,0,0]) + payload
    c = crc16(body[1:], extra)
    ctl.write(body + bytes([c&0xff, c>>8]))

def send_prl(extra=159):
    payload = bytes([1,1])
    body = bytes([0xFD, len(payload), 0,0,0, 1,1, 21,0,0]) + payload
    c = crc16(body[1:], extra)
    ctl.write(body + bytes([c&0xff, c>>8]))

send_prl()
send_cl(520, 1.0)
send_cl(400, 1.0)

# 监听 6 秒，统计下行 msgid
hist = Counter()
t0 = time.time()
while time.time() - t0 < 6:
    b = ctl.read(300)
    if not b:
        continue
    i = 0
    while i + 9 < len(b):
        if b[i] == 0xFD:
            ln = b[i+1]
            mid = b[i+7] | (b[i+8]<<8) | (b[i+9]<<16)
            tot = 10 + ln + 2
            if i + tot <= len(b):
                hist[mid] += 1
                i += tot
                continue
        i += 1

print('received msgid histogram:')
for k in sorted(hist):
    print('  msgid %d (0x%02X): %d' % (k, k, hist[k]))
print('CAP(148) count =', hist.get(148,0))
print('PARAM_VALUE(22) count =', hist.get(22,0))
print('COMMAND_ACK(77) count =', hist.get(77,0))
ctl.close()
