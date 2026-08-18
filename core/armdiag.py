import serial, time, sys, threading

# 同时监听 COM8(板子调试日志) 并往 COM12(USB CDC) 发 ARM 命令，定位 ARM 是否到达板子。
listener = serial.Serial('COM8', 115200, timeout=0.2)
listener.dtr = False
listener.rts = False

def listen():
    t0 = time.time()
    with open('armdiag_com8.txt', 'w') as f:
        while time.time() - t0 < 8:
            try:
                b = listener.read(200)
            except Exception:
                break
            if b:
                txt = b.decode('utf-8', 'replace')
                f.write(txt)
                f.flush()
    listener.close()

th = threading.Thread(target=listen, daemon=True)
th.start()
time.sleep(0.5)

# 发 ARM 命令 (COMMAND_LONG cmd=400 p1=1)
ctl = serial.Serial('COM12', 115200, timeout=1)
ctl.dtr = False
ctl.rts = False
time.sleep(0.3)

def send_cl(command, p1=0.0):
    import struct
    payload = bytes([1,1, command&0xff, command>>8, 0]) + b''.join(struct.pack('<f', x) for x in [p1,0,0,0,0,0,0]) + bytes([0])
    body = bytes([0xFD, len(payload), 0,0,0, 1,1, 76,0,0]) + payload
    # crc16 x25 reflected with extra 152
    crc = 0xFFFF
    for x in list(body[1:]) + [152]:
        crc ^= x
        for _ in range(8):
            crc = (crc>>1) ^ 0x8408 if (crc&1) else (crc>>1)
    ctl.write(body + bytes([crc&0xff, crc>>8]))
    print('sent ARM cmd=%d p1=%.1f' % (command, p1))

send_cl(400, 1.0)
time.sleep(2)
send_cl(400, 1.0)
time.sleep(2)
ctl.close()
th.join()
print('done; COM8 log -> armdiag_com8.txt')
