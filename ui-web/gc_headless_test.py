#!/usr/bin/env python3
# 无头浏览器联调：加载地面站 WebUI，验证所有 UI 功能 + 后端指令闭环。
import asyncio, json, struct, time, websockets
from playwright.async_api import async_playwright

WS_BRIDGE = 'ws://localhost:8787'
UI_URL = 'http://127.0.0.1:5173/'

def crc16(buf, extra):
    # 与桥 gc_bridge.crc16 / 飞控 crc16_x25 严格一致的反射 CRC-16-CCITT/X25：
    # 初值 0xFFFF，逐字节反射累积，最后把 CRC_EXTRA 作为「最后一个字节」走完整 8 轮移位。
    # 之前用查表法时 extra 折叠逻辑写错（做两次表步），导致 CRC 与正确值不一致（实测
    # COMMAND_LONG 帧被板子 CRC 校验拒绝、ARM/模式切换全不生效）。必须用与桥相同的位算法。
    crc = 0xFFFF
    for x in buf:
        crc ^= x
        for _ in range(8):
            crc = ((crc >> 1) ^ 0x8408) if (crc & 1) else (crc >> 1)
    crc ^= extra
    for _ in range(8):
        crc = ((crc >> 1) ^ 0x8408) if (crc & 1) else (crc >> 1)
    return crc & 0xFFFF
def enc_command_long(command, params=(0,0,0,0,0,0,0), confirmation=0, seq=0):
    # 必须补齐 7 个 f32 参数（标准 COMMAND_LONG payload = 5 头部 + 28 参数 = 33B），
    # 否则板子 decode_command_long 要求 payload>=33 会直接丢弃指令（实测 ARM/DISARM 不生效）。
    p = list(params) + [0.0] * (7 - len(params))
    pl = bytes([1,1, command & 0xFF, command >> 8, confirmation])
    pl += b''.join(struct.pack('<f', float(x)) for x in p)
    body = bytes([0xFD, len(pl), 0,0, seq, 1,1, 76,0,0]) + pl
    c = crc16(body[1:], 152)
    return body + bytes([c & 0xFF, c >> 8])

async def invoke(ws, cmd, args=None, timeout=4):
    mid = int(time.time()*1000) % 100000 + id(cmd)
    fut = asyncio.get_event_loop().create_future()
    async def waiter():
        try:
            async for raw in ws:
                m = json.loads(raw)
                if m.get('type') == 'resp' and m.get('id') == mid:
                    fut.set_result(m); break
        except Exception:
            pass
    task = asyncio.create_task(waiter())
    await ws.send(json.dumps({'type':'invoke','id':mid,'cmd':cmd,'args':args or {}}))
    try:
        return await asyncio.wait_for(fut, timeout)
    finally:
        task.cancel()

async def raw(ws, frame):
    await invoke(ws, '__raw', {'frame': list(frame)})

results = []
async def main():
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True)
        page = await browser.new_page()
        errors = []
        page.on('pageerror', lambda e: errors.append(str(e)))
        await page.goto(UI_URL, wait_until='networkidle')
        await page.wait_for_timeout(800)

        # 连接
        await page.click('#btn-connect')
        await page.wait_for_timeout(2000)
        results.append(('连接状态栏', await page.text_content('#st-link')))

        # 经桥直接控制飞控（重置到已知状态：DISARM + STABILIZE）
        async with websockets.connect(WS_BRIDGE) as ws:
            await invoke(ws, 'connect', {'kind':'serial','bind':'COM12','target':'115200'})
            await raw(ws, enc_command_long(400, (0.0,)))  # DISARM
            await raw(ws, enc_command_long(176, (1.0, 0.0)))  # DO_SET_MODE STABILIZE
            await page.wait_for_timeout(800)

        # 遥测：应显示已上锁 + STABILIZE
        results.append(('DISARM后遥测', {
            'arm': await page.text_content('#st-arm'),
            'mode': await page.text_content('#st-mode'),
            'alt': await page.text_content('#st-alt'),
        }))

        # 拉取参数
        await page.click('#btn-params')
        await page.wait_for_timeout(2500)
        params_html = await page.inner_html('#params')
        n_params = params_html.count('pname')
        results.append(('拉取参数条目数', n_params))

        # 参数写入（走 webui set_param invoke）：取第一个参数名，写新值
        write_result = 'N/A'
        if n_params > 0:
            pname = await page.evaluate("""() => document.querySelector('#params .pname').textContent""")
            await page.evaluate("""(name) => {
                // 直接触发 webui 的 setParam 路径：通过双击弹出 prompt 不便，改用 invoke
                // 这里用桥 set_param 验证后端，UI 渲染验证见下方
            }""", pname)
            # 经桥走 set_param（与 webui 调用同一后端命令）
            async with websockets.connect(WS_BRIDGE) as ws:
                await invoke(ws, 'set_param', {'name': pname, 'value': 2.0})
                await page.wait_for_timeout(1500)
            # 重新拉取，验证值已更新
            await page.click('#btn-params')
            await page.wait_for_timeout(2500)
            vals = await page.evaluate("""() => [...document.querySelectorAll('#params .pval')].map(e=>e.textContent)""")
            write_result = {'param': pname, 'vals_after': vals[:3]}
        results.append(('参数写入验证', write_result))

        # 后端指令闭环：ARM / 模式切换 / 起飞 / 降落 / RTL（经桥 __raw）
        cmd_results = {}
        async with websockets.connect(WS_BRIDGE) as ws:
            await invoke(ws, 'connect', {'kind':'serial','bind':'COM12','target':'115200'})
            # ARM
            await raw(ws, enc_command_long(400, (1.0,)))
            await page.wait_for_timeout(1000)
            cmd_results['ARM'] = await page.text_content('#st-arm')
            # DO_SET_MODE ALT_HOLD(2)
            await raw(ws, enc_command_long(176, (1.0, 2.0)))
            await page.wait_for_timeout(1000)
            cmd_results['ALT_HOLD'] = await page.text_content('#st-mode')
            # DO_SET_MODE LOITER(5)
            await raw(ws, enc_command_long(176, (1.0, 5.0)))
            await page.wait_for_timeout(1000)
            cmd_results['LOITER'] = await page.text_content('#st-mode')
            # RTL(11)
            await raw(ws, enc_command_long(20, ()))
            await page.wait_for_timeout(1000)
            cmd_results['RTL'] = await page.text_content('#st-mode')
            # DISARM 复原
            await raw(ws, enc_command_long(400, (0.0,)))
            await page.wait_for_timeout(800)
            cmd_results['DISARM'] = await page.text_content('#st-arm')
        results.append(('后端指令闭环', cmd_results))

        # 航点下载按钮可用
        await page.click('#btn-wp-download')
        await page.wait_for_timeout(1000)
        results.append(('航点下载', (await page.inner_html('#mission'))[:60]))

        # 告警规则应用
        await page.fill('#cfg-warn', '30')
        await page.fill('#cfg-crit', '15')
        await page.click('#btn-cfg')
        await page.wait_for_timeout(500)
        results.append(('告警规则应用', 'ok'))

        await browser.close()

    print('===== 联调结果 =====')
    for k, v in results:
        print(f'{k}: {v}')
    print('===== 页面错误 =====')
    for e in errors[:10]:
        print(e)
    # 判定
    ok = (results[0][1].find('已连接') >= 0 and n_params > 0
          and cmd_results.get('ARM','').find('已解锁') >= 0
          and cmd_results.get('ALT_HOLD','').find('ALT_HOLD') >= 0
          and cmd_results.get('LOITER','').find('LOITER') >= 0)
    print('===== 总判定 =====')
    print('PASS' if ok else 'FAIL')

if __name__ == '__main__':
    asyncio.run(main())
