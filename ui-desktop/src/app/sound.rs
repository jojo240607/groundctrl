//! 声音告警（P2-4）：用 rodio 合成正弦提示音，按告警等级区分音调。
//! 无音频设备时静默降级（返回 None）。

use std::time::Duration;

use groundctrl_core::services::alarms::AlarmLevel;
use rodio::{source::SineWave, OutputStream, Sink, Source};

pub struct Sounder {
    _stream: OutputStream,
    sink: Sink,
}

impl Sounder {
    /// 初始化音频输出；无可用声卡时返回 None（静默模式）
    pub fn new() -> Option<Self> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        let sink = Sink::try_new(&handle).ok()?;
        Some(Self {
            _stream: stream,
            sink,
        })
    }

    /// 播放单音：freq = 频率（Hz），ms = 时长
    pub fn beep(&self, freq: f32, ms: u64) {
        let src = SineWave::new(freq)
            .take_duration(Duration::from_millis(ms))
            .amplify(0.3);
        self.sink.append(src);
    }

    /// 按告警等级播放提示音：
    /// - Critical：880Hz 双短音（急促）
    /// - Warn：660Hz 单音
    /// - Info：440Hz 短音
    pub fn alarm(&self, level: AlarmLevel) {
        match level {
            AlarmLevel::Critical => {
                self.beep(880.0, 180);
                self.beep(880.0, 180);
            }
            AlarmLevel::Warn => self.beep(660.0, 200),
            AlarmLevel::Info => self.beep(440.0, 150),
        }
    }
}
