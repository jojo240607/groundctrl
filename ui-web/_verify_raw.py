import serial, time, struct

def crc16(buf, extra):
    table = []
    for i in range(256):
        c=i
        for _ in range(8):
            c=(c>>1)^0x8408 if c&1 else c>>1
        table.append(c)
    crc=0xFFFF
    for x in buf:
        crc=(crc>>8)^table[((crc^x)&0xFF)]
    crc^=extra
    crc=(crc>>8)^table[(crc&0xFF)^0]
    crc=(crc>>8)^table[(crc&0xFF)]
    return crc

def cmd_long(command, params=(0,0,0,0,0,0,0), seq=0):
    pl=bytes([1,1,command&0xFF,command>>8,0])+b''.join(struct.pack('<f',float(x)) for x in params)
    body=bytes([0xFD,len(pl),0,0,seq,1,1,76,0,0])+pl
    c=crc16(body[1:],152); return body+bytes([c&0xFF,c>>8])

def preq(seq=0):
    pl=bytes([1,1]); body=bytes([0xFD,len(pl),0,0,seq,1,1,21,0,0])+pl
    c=crc16(body[1:],159); return body+bytes([c&0xFF,c>>8])

s=serial.Serial('COM12',115200,timeout=0.3); s.dtr=False; s.rts=False; time.sleep(0.3)
s.reset_input_buffer()
s.write(cmd_long(400,(1.0,)))  # ARM
s.write(preq())                # request params
t0=time.time(); seen={}
while time.time()-t0<4:
    b=s.read(512); i=0
    while i+9<len(b):
        if b[i]==0xFD:
            ln=b[i+1]; mid=b[i+7]|(b[i+8]<<8)|(b[i+9]<<16); tot=10+ln+2
            if i+tot<=len(b):
                seen[mid]=seen.get(mid,0)+1
                if mid==0: base=b[i+12]; print('HEARTBEAT base_mode=%d (armed=%s)'%(base, bool(base&0x80)))
                i+=tot; continue
        i+=1
print('msgid_hist=',dict(sorted(seen.items())))
