import serial, time, sys
try:
    s = serial.Serial('COM12', 115200, timeout=1)
    s.dtr = False
    s.rts = False
    print('open OK')
    frame = bytes([0xFD,0x02,0x00,0x00,0x00,0x01,0x01,0x15,0x00,0x00,0x01,0x01])
    print('frame', frame.hex())
    s.write(frame)
    time.sleep(1)
    data = s.read(200)
    print('read', len(data), data.hex())
    s.close()
except Exception as e:
    print('ERR', repr(e))
    sys.exit(1)
