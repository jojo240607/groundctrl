//! 脚本任务（P3-4）：迷你任务 DSL 的解析与执行
//!
//! 语法（每行一条指令，`#` 开头为注释，指令不区分大小写）：
//!
//! ```text
//! # 示例：起飞 → 巡航 → 返航
//! LOG 任务开始
//! SET_MODE GUIDED          # 或 SET_MODE 4
//! ARM
//! WAIT 2                   # 等待 2 秒
//! TAKEOFF 10               # 起飞到 10m
//! IF BATTERY_LT 30         # 电量低于 30%
//!   RTL
//! END
//! IF GPS_LT 3              # GPS 精度低于 3D 定位
//!   LOG 无 3D 定位
//! END
//! IF ARMED                 # 当前已解锁
//!   DISARM
//! END
//! LAND
//! DISARM
//! ```
//!
//! 执行时通过 [`TelemetryHub`] 下发 MAVLink 指令；条件判断基于
//! `get_vehicle` 闭包返回的实时快照（UI 传入当前机队快照，测试可注入确定值）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ::mavlink::common as mav;

use crate::error::{GcError, Result};
use crate::services::TelemetryHub;
use crate::vehicle::VehicleModel;

/// 脚本中止信号（可 Clone，跨任务共享）
#[derive(Clone, Default)]
pub struct ScriptAbort(Arc<AtomicBool>);

impl ScriptAbort {
    pub fn new() -> Self {
        Self::default()
    }

    /// 请求中止（执行器在语句边界检查该信号）
    pub fn abort(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// 是否已请求中止
    pub fn is_aborted(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// 执行过程中的事件（供 UI 展示）
#[derive(Debug, Clone)]
pub enum ScriptEvent {
    /// 执行到某条语句（块内索引, 描述文本）
    Step(usize, String),
    /// LOG 输出
    Log(String),
}

/// IF 条件
#[derive(Debug, Clone, PartialEq)]
pub enum IfCond {
    /// 电池余电低于阈值（%）
    BatteryLt(f32),
    /// GPS 定位精度低于级别（fix_type：1=无定位 2=2D 3=3D；`GPS_LT 3` = 没有 3D 定位）
    GpsLt(u8),
    /// 当前已解锁
    Armed,
}

/// 脚本语句
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// 等待若干秒（可中止）
    WaitSecs(f32),
    /// 切换模式（ArduPilot Copter custom_mode）
    SetMode(u32),
    Arm,
    Disarm,
    /// 起飞到指定高度（米）
    Takeoff(f32),
    Land,
    Rtl,
    /// 跳转到航点（索引 0 起，MAV_CMD_DO_SET_MISSION_CURRENT）
    GotoWp(u16),
    /// 写参数（SET_PARAM 名称 值）
    SetParam(String, f32),
    /// 输出日志
    Log(String),
    /// 条件分支
    If(IfCond, Vec<Stmt>),
}

/// 已解析的脚本程序
#[derive(Debug, Clone)]
pub struct ScriptProgram {
    pub name: String,
    pub stmts: Vec<Stmt>,
}

/// 解析脚本文本（`#` 注释 / 空行跳过；IF 块以 END 或文本末尾闭合）
pub fn parse_script(name: &str, text: &str) -> Result<ScriptProgram> {
    let lines: Vec<&str> = text.lines().collect();
    let (stmts, used) = parse_block(&lines, 0)?;
    // 顶层剩余的非空行只能是多余的 END
    for (k, l) in lines.iter().enumerate().skip(used) {
        let t = l.trim();
        if !t.is_empty() && !t.starts_with('#') {
            return Err(line_err(k + 1, "多余的 END（无对应的 IF）"));
        }
    }
    Ok(ScriptProgram {
        name: name.to_string(),
        stmts,
    })
}

/// 递归解析语句块；返回（语句, 下一行索引）。遇到 END 或文本末尾结束。
fn parse_block(lines: &[&str], start: usize) -> Result<(Vec<Stmt>, usize)> {
    let mut stmts = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let line = lines[i].trim();
        let lineno = i + 1;
        i += 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "END" {
            return Ok((stmts, i));
        }
        if let Some(rest) = line.strip_prefix("IF ") {
            let cond = parse_if_cond(rest, lineno)?;
            let (sub, ni) = parse_block(lines, i)?;
            i = ni;
            stmts.push(Stmt::If(cond, sub));
            continue;
        }
        stmts.push(parse_stmt(line, lineno)?);
    }
    Ok((stmts, i))
}

/// 解析单条语句（不含 IF/END）
fn parse_stmt(line: &str, lineno: usize) -> Result<Stmt> {
    let mut it = line.split_whitespace();
    let cmd = it.next().unwrap_or("").to_ascii_uppercase();
    let args: Vec<&str> = it.collect();
    match cmd.as_str() {
        "WAIT" => {
            let v = one_arg(&args, lineno, "WAIT 需要时间（秒）")?;
            let secs: f32 = v
                .parse()
                .map_err(|_| line_err(lineno, &format!("WAIT 时间无效: {v}")))?;
            Ok(Stmt::WaitSecs(secs))
        }
        "SET_MODE" => {
            let v = one_arg(&args, lineno, "SET_MODE 需要模式")?;
            if let Ok(m) = v.parse::<u32>() {
                Ok(Stmt::SetMode(m))
            } else if let Some(m) = mode_number(v) {
                Ok(Stmt::SetMode(m))
            } else {
                Err(line_err(lineno, &format!("SET_MODE 模式无效: {v}")))
            }
        }
        "ARM" => Ok(Stmt::Arm),
        "DISARM" => Ok(Stmt::Disarm),
        "TAKEOFF" => {
            let v = one_arg(&args, lineno, "TAKEOFF 需要高度（米）")?;
            let alt: f32 = v
                .parse()
                .map_err(|_| line_err(lineno, &format!("TAKEOFF 高度无效: {v}")))?;
            Ok(Stmt::Takeoff(alt))
        }
        "LAND" => Ok(Stmt::Land),
        "RTL" => Ok(Stmt::Rtl),
        "GOTO_WP" => {
            let v = one_arg(&args, lineno, "GOTO_WP 需要航点索引")?;
            let idx: u16 = v
                .parse()
                .map_err(|_| line_err(lineno, &format!("GOTO_WP 索引无效: {v}")))?;
            Ok(Stmt::GotoWp(idx))
        }
        "SET_PARAM" => {
            let name = args
                .first()
                .ok_or_else(|| line_err(lineno, "SET_PARAM 需要参数名和值"))?;
            let val = args
                .get(1)
                .ok_or_else(|| line_err(lineno, &format!("SET_PARAM {name} 缺少值")))?;
            let value: f32 = val
                .parse()
                .map_err(|_| line_err(lineno, &format!("SET_PARAM 值无效: {val}")))?;
            Ok(Stmt::SetParam(name.to_string(), value))
        }
        "LOG" => {
            if args.is_empty() {
                return Err(line_err(lineno, "LOG 需要文本"));
            }
            Ok(Stmt::Log(args.join(" ")))
        }
        _ => Err(line_err(lineno, &format!("未知指令: {cmd}"))),
    }
}

fn one_arg<'a>(args: &'a [&'a str], lineno: usize, msg: &str) -> Result<&'a str> {
    args.first().copied().ok_or_else(|| line_err(lineno, msg))
}

/// 解析 IF 条件
fn parse_if_cond(rest: &str, lineno: usize) -> Result<IfCond> {
    let mut it = rest.split_whitespace();
    let kind = it.next().unwrap_or("").to_ascii_uppercase();
    let v = it.next();
    match kind.as_str() {
        "BATTERY_LT" => {
            let s = v.ok_or_else(|| line_err(lineno, "IF BATTERY_LT 需要阈值（%）"))?;
            let pct: f32 = s
                .parse()
                .map_err(|_| line_err(lineno, &format!("IF BATTERY_LT 阈值无效: {s}")))?;
            Ok(IfCond::BatteryLt(pct))
        }
        "GPS_LT" => {
            let s = v.ok_or_else(|| line_err(lineno, "IF GPS_LT 需要级别（2=2D 3=3D）"))?;
            let lv: u8 = s
                .parse()
                .map_err(|_| line_err(lineno, &format!("IF GPS_LT 级别无效: {s}")))?;
            Ok(IfCond::GpsLt(lv))
        }
        "ARMED" => Ok(IfCond::Armed),
        _ => Err(line_err(lineno, &format!("未知条件: {kind}"))),
    }
}

/// ArduPilot Copter custom_mode 名称表（SET_MODE 支持名称或数字）
fn mode_number(name: &str) -> Option<u32> {
    let n = name.to_ascii_uppercase();
    let m = match n.as_str() {
        "STABILIZE" => 0,
        "ACRO" => 1,
        "ALT_HOLD" => 2,
        "AUTO" => 3,
        "GUIDED" => 4,
        "LOITER" => 5,
        "RTL" => 6,
        "CIRCLE" => 7,
        "LAND" => 9,
        "DRIFT" => 11,
        "SPORT" => 13,
        "FLIP" => 14,
        "AUTOTUNE" => 15,
        "POSHOLD" => 16,
        "BRAKE" => 17,
        "THROW" => 18,
        "SMART_RTL" => 21,
        "FLOWHOLD" => 22,
        "FOLLOW" => 23,
        "ZIGZAG" => 24,
        "AUTOROTATE" => 26,
        "AUTO_RTL" => 27,
        _ => return None,
    };
    Some(m)
}

fn line_err(lineno: usize, msg: &str) -> GcError {
    GcError::Script(format!("第 {lineno} 行: {msg}"))
}

/// 运行脚本（语句按序执行，语句边界检查中止信号）
///
/// - `get_vehicle`：每次条件判断时调用，返回当前飞机快照（None = 无数据，条件视为假）
/// - `on_event`：执行步骤 / 日志回调
pub async fn run_script(
    hub: &TelemetryHub,
    get_vehicle: impl Fn() -> Option<VehicleModel> + Send + Sync,
    prog: &ScriptProgram,
    abort: &ScriptAbort,
    on_event: &mut (dyn FnMut(ScriptEvent) + Send),
) -> Result<()> {
    run_block(hub, &get_vehicle, &prog.stmts, abort, on_event).await
}

/// 递归执行语句块（IF 子块嵌套；返回 boxed future 以支持 async 递归）
fn run_block<'a>(
    hub: &'a TelemetryHub,
    get_vehicle: &'a (impl Fn() -> Option<VehicleModel> + Send + Sync),
    stmts: &'a [Stmt],
    abort: &'a ScriptAbort,
    on_event: &'a mut (dyn FnMut(ScriptEvent) + Send),
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        for (idx, stmt) in stmts.iter().enumerate() {
            if abort.is_aborted() {
                return Err(GcError::Script("脚本已中止".into()));
            }
            match stmt {
                Stmt::WaitSecs(secs) => {
                    on_event(ScriptEvent::Step(idx, format!("WAIT {secs} 秒")));
                    // 100ms 一片等待，便于及时响应中止
                    let ticks = (*secs * 10.0).round().max(0.0) as u32;
                    for _ in 0..ticks {
                        if abort.is_aborted() {
                            return Err(GcError::Script("脚本已中止".into()));
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
                Stmt::SetMode(m) => {
                    on_event(ScriptEvent::Step(idx, format!("SET_MODE {m}")));
                    hub.send_mode(1, 1, *m).await?;
                }
                Stmt::Arm => {
                    on_event(ScriptEvent::Step(idx, "ARM".into()));
                    hub.send_arm_disarm(1, 1, true).await?;
                }
                Stmt::Disarm => {
                    on_event(ScriptEvent::Step(idx, "DISARM".into()));
                    hub.send_arm_disarm(1, 1, false).await?;
                }
                Stmt::Takeoff(alt) => {
                    on_event(ScriptEvent::Step(idx, format!("TAKEOFF {alt} 米")));
                    hub.send_takeoff(1, 1, *alt).await?;
                }
                Stmt::Land => {
                    on_event(ScriptEvent::Step(idx, "LAND".into()));
                    hub.send_land(1, 1).await?;
                }
                Stmt::Rtl => {
                    on_event(ScriptEvent::Step(idx, "RTL".into()));
                    hub.send_rtl(1, 1).await?;
                }
                Stmt::GotoWp(i) => {
                    on_event(ScriptEvent::Step(idx, format!("GOTO_WP {i}")));
                    hub.send_command_long(
                        1,
                        1,
                        mav::MavCmd::MAV_CMD_DO_SET_MISSION_CURRENT,
                        [*i as f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                    )
                    .await?;
                }
                Stmt::SetParam(name, value) => {
                    on_event(ScriptEvent::Step(idx, format!("SET_PARAM {name} {value}")));
                    hub.set_param(1, 1, name, *value).await?;
                }
                Stmt::Log(text) => {
                    on_event(ScriptEvent::Step(idx, format!("LOG {text}")));
                    on_event(ScriptEvent::Log(text.clone()));
                }
                Stmt::If(cond, sub) => {
                    let hit = eval_cond(cond, get_vehicle());
                    on_event(ScriptEvent::Step(idx, format!("IF {cond:?} => {hit}")));
                    if hit {
                        run_block(hub, get_vehicle, sub, abort, on_event).await?;
                    }
                }
            }
        }
        Ok(())
    })
}

/// 计算条件（无车辆数据或字段未知时视为假）
fn eval_cond(cond: &IfCond, v: Option<VehicleModel>) -> bool {
    let Some(v) = v else { return false };
    match cond {
        IfCond::BatteryLt(pct) => v
            .battery
            .remaining_pct
            .map(|p| (p as f32) < *pct)
            .unwrap_or(false),
        IfCond::GpsLt(level) => v.gps.fix_type < *level,
        IfCond::Armed => v.is_armed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_and_if_block() {
        let prog = parse_script(
            "t",
            "# 注释\nWAIT 2.5\nSET_MODE GUIDED\nIF BATTERY_LT 30\n  RTL\nEND\nARM",
        )
        .expect("解析应成功");
        assert_eq!(prog.name, "t");
        assert_eq!(prog.stmts.len(), 4);
        assert_eq!(prog.stmts[0], Stmt::WaitSecs(2.5));
        assert_eq!(prog.stmts[1], Stmt::SetMode(4));
        assert_eq!(
            prog.stmts[2],
            Stmt::If(IfCond::BatteryLt(30.0), vec![Stmt::Rtl])
        );
        assert_eq!(prog.stmts[3], Stmt::Arm);
    }

    #[test]
    fn parse_errors_have_line_numbers() {
        let err = parse_script("t", "WAIT abc").expect_err("参数无效应报错");
        assert!(err.to_string().contains("1"), "{err}");
        let err = parse_script("t", "ARM\nFOO 1").expect_err("未知指令应报错");
        assert!(err.to_string().contains("FOO"), "{err}");
        assert!(err.to_string().contains("2"), "{err}");
        let err = parse_script("t", "ARM\nEND\nRTL").expect_err("多余 END 应报错");
        assert!(err.to_string().contains("END"), "{err}");
        let err = parse_script("t", "SET_MODE BOGUS").expect_err("无效模式应报错");
        assert!(err.to_string().contains("BOGUS"), "{err}");
        let err = parse_script("t", "IF GPS_LT").expect_err("缺条件参数应报错");
        assert!(err.to_string().contains("级别"), "{err}");
        let err = parse_script("t", "IF BOGUS 1").expect_err("未知条件应报错");
        assert!(err.to_string().contains("BOGUS"), "{err}");
    }

    #[test]
    fn unclosed_if_allowed_until_eof() {
        let ok = parse_script("t", "IF ARMED\nRTL").expect("未闭合 IF 应允许");
        assert_eq!(ok.stmts.len(), 1);
        assert!(matches!(&ok.stmts[0], Stmt::If(IfCond::Armed, s) if s.len() == 1));
    }
}
