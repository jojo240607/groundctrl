import serial, time, struct

def crc16(buf, extra):
    # 严格匹配飞控 flyctrl_core::comm::mavlink::crc16_x25（反射 0x8408，无字节交换）
    crc=0xFFFF
    for x in buf:
        crc^=x
        for _ in range(8):
            crc=((crc>>1)^0x8408) if (crc&1) else (crc>>1)
    crc^=extra
    return crc&0xFFFF

def cmd_long(command, params=(0,0,0,0,0,0,0), seq=0):
    pl=bytes([1,1,command&0xFF,command>>8,0])+b''.join(struct.pack('<f',float(x)) for x in params)
    body=bytes([0xFD,len(pl),0,0,seq,1,1,76,0,0])+pl
    c=crc16(body[1:],152); return body+bytes([c&0xFF,c>>8])

def read_base_mode(ser, dur=3.0):
    ser.reset_input_buffer()
    t0=time.time(); bm=None; seen={}
    while time.time()-t0<dur:
        b=ser.read(512); i=0
        while i+12<len(b):
            if b[i]==0xFD:
                ln=b[i+1]; mid=b[i+7]|(b[i+8]<<8)|(b[i+9]<<16); tot=10+ln+2
                if i+tot<=len(b):
                    seen[mid]=seen.get(mid,0)+1
                    if mid==0:
                        cm=int.from_bytes(b[i+10:i+14],'little')
                        bm=b[i+16]  # base_mode = payload[6]
                    i+=tot; continue
            i+=1
    return bm, seen

s=serial.Serial('COM12',115200,timeout=0.3); s.dtr=False; s.rts=False; time.sleep(0.3)

print('初始 base_mode =', read_base_mode(s))

def send_and_check(ser,command,params,label):
    ser.write(cmd_long(command,params))
    # 监听下行 2 秒：看有无 COMMAND_ACK(77) + 心跳变化
    ser.reset_input_buffer(); t0=time.time(); ack=False; bm=None; cm=None
    while time.time()-t0<2.0:
        b=ser.read(512); i=0
        while i+10<len(b):
            if b[i]==0xFD:
                ln=b[i+1]; mid=b[i+7]|(b[i+8]<<8)|(b[i+9]<<16); tot=10+ln+2
                if i+tot<=len(b):
                    if mid==77: ack=True
                    if mid==0:
                        cm=int.from_bytes(b[i+10:i+14],'little')
                        bm=b[i+16]
                    i+=tot; continue
            i+=1
    print('%s -> ACK=%s base_mode=%s(custom=%s)'%(label,ack,bm,cm))

send_and_check(s,400,(0.0,),'DISARM')
send_and_check(s,400,(1.0,),'ARM')
send_and_check(s,176,(1.0,2.0),'DO_SET_MODE ALT_HOLD')
send_and_check(s,176,(1.0,4.0),'DO_SET_MODE LOITER')
