use crate::errors::ExtensionError;
use std::time::Duration;

mod abs_send_time_extension_test;

const ABS_SEND_TIME_EXTENSION_SIZE: usize = 3;

/// AbsSendTimeExtension is a extension payload format in
/// http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time
#[derive(Debug, Default)]
pub struct AbsSendTimeExtension {
    pub timestamp: u64,
}

impl AbsSendTimeExtension {
    /// Marshal serializes the members to buffer.
    pub fn marshal(&self) -> Result<Vec<u8>, ExtensionError> {
        Ok(vec![
            ((self.timestamp & 0xFF0000) >> 16) as u8,
            ((self.timestamp & 0xFF00) >> 8) as u8,
            (self.timestamp & 0xFF) as u8,
        ])
    }

    /// Unmarshal parses the passed byte slice and stores the result in the members.
    pub fn unmarshal(&mut self, raw_data: &[u8]) -> Result<(), ExtensionError> {
        if raw_data.len() < ABS_SEND_TIME_EXTENSION_SIZE {
            return Err(ExtensionError::TooSmall);
        }
        self.timestamp =
            (raw_data[0] as u64) << 16 | (raw_data[1] as u64) << 8 | raw_data[2] as u64;

        Ok(())
    }

    /// Estimate absolute send time according to the receive time.
    /// Note that if the transmission delay is larger than 64 seconds, estimated time will be wrong.
    pub fn estimate(&self, receive: Duration) -> Duration {
        let receive_ntp = unix2ntp(receive);
        let mut ntp = receive_ntp & 0xFFFFFFC000000000 | (self.timestamp & 0xFFFFFF) << 14;
        if receive_ntp < ntp {
            // Receive time must be always later than send time
            ntp -= 0x1000000 << 14;
        }

        ntp2unix(ntp)
    }

    /// NewAbsSendTimeExtension makes new AbsSendTimeExtension from time.Time.
    pub fn new(send_time: Duration) -> Self {
        AbsSendTimeExtension {
            timestamp: unix2ntp(send_time) >> 14,
        }
    }
}

pub fn unix2ntp(t: Duration) -> u64 {
    let u = t.as_nanos() as u64;
    let mut s = u / 1_000_000_000;
    s += 0x83AA7E80; //offset in seconds between unix epoch and ntp epoch
    let mut f = u % 1_000_000_000;
    f <<= 32;
    f /= 1_000_000_000;
    s <<= 32;

    s | f
}

pub fn ntp2unix(t: u64) -> Duration {
    let mut s = t >> 32;
    let mut f = t & 0xFFFFFFFF;
    f *= 1_000_000_000;
    f >>= 32;
    s -= 0x83AA7E80;
    let u = s * 1_000_000_000 + f;

    Duration::new(u / 1_000_000_000, (u % 1_000_000_000) as u32)
}
