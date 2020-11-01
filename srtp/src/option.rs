use super::context::srtcp::*;
use super::context::*;
use transport::replay_detector::*;

pub type ContextOption = Box<dyn Fn() -> Box<dyn ReplayDetector + Send>>;

// srtp_replay_protection sets SRTP replay protection window size.
pub fn srtp_replay_protection(window_size: usize) -> ContextOption {
    Box::new(move || -> Box<dyn ReplayDetector + Send> {
        Box::new(WrappedSlidingWindowDetector::new(
            window_size,
            MAX_SEQUENCE_NUMBER as u64,
        ))
    })
}

// SRTCPReplayProtection sets SRTCP replay protection window size.
pub fn srtcp_replay_protection(window_size: usize) -> ContextOption {
    Box::new(move || -> Box<dyn ReplayDetector + Send> {
        Box::new(WrappedSlidingWindowDetector::new(
            window_size,
            MAX_SRTCP_INDEX,
        ))
    })
}

// srtp_no_replay_protection disables SRTP replay protection.
pub fn srtp_no_replay_protection() -> ContextOption {
    Box::new(|| -> Box<dyn ReplayDetector + Send> { Box::new(NoOpReplayDetector::default()) })
}

// srtcp_no_replay_protection disables SRTCP replay protection.
pub fn srtcp_no_replay_protection() -> ContextOption {
    Box::new(|| -> Box<dyn ReplayDetector + Send> { Box::new(NoOpReplayDetector::default()) })
}
