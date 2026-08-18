import serial, time, struct
ctl = serial.Serial('COM12', 115200, timeout=1)
ctl.dtr = False; ctl.rts = False
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

def send_cl(command, p1=0.0):
    payload = bytes([1,1, command&0xff, command>>8, 0]) + b''.join(struct.pack('<f', x) for x in [p1,0,0,0,0,0,0]) + bytes([0])
    body = bytes([0xFD, len(payload), 0,0,0, 1,1, 76,0,0]) + payload
    c = crc16(body[1:], 152)
    ctl.write(body + bytes([c&0xff, c>>8]))

# 先看解锁前 base_mode
def scan_base_mode(dur):
    t0 = time.time(); modes = set()
    while time.time() - t0 < dur:
        b = ctl.read(400)
        i = 0
        while i + 9 < len(b):
            if b[i] == 0xFD:
                ln = b[i+1]; mid = b[i+7]|(b[i+8]<<8)|(b[i+9]<<16); tot = 10+ln+2
                if i+tot <= len(b):
                    if mid == 0:  # heartbeat
                        base = b[i+10+0]  # base_mode at payload offset 0
                        modes.add(base)
                    if mid == 77:  # ACK
                        cmd = b[i+10]|(b[i+11]<<8); res = b[i+10+2]
                        print('  ACK cmd=%d result=%d' % (cmd, res))
                    i += tot; continue
            i += 1
    return modes

print('before ARM base_mode set:', scan_base_mode(2))
send_cl(400, 1.0)
time.sleep(0.5)
print('after ARM base_mode set:', scan_base_mode(3))
ctl.close()
