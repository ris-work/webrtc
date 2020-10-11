use std::collections::HashMap;

use transport::replay_detector::*;
use util::Error;

use super::cipher::*;
use super::protection_profile::*;
use crate::cipher::cipher_aes_cm_hmac_sha1::CipherAesCmHmacSha1;
use crate::option::*;

#[cfg(test)]
mod context_test;

#[cfg(test)]
mod srtp_test;

#[cfg(test)]
mod srtcp_test;

pub mod srtcp;
pub mod srtp;

pub const LABEL_SRTP_ENCRYPTION: u8 = 0x00;
pub const LABEL_SRTP_AUTHENTICATION_TAG: u8 = 0x01;
pub const LABEL_SRTP_SALT: u8 = 0x02;
pub const LABEL_SRTCP_ENCRYPTION: u8 = 0x03;
pub const LABEL_SRTCP_AUTHENTICATION_TAG: u8 = 0x04;
pub const LABEL_SRTCP_SALT: u8 = 0x05;

const MAX_ROC_DISORDER: u16 = 100;
pub(crate) const MAX_SEQUENCE_NUMBER: u16 = 65535;
pub(crate) const SRTCP_INDEX_SIZE: usize = 4;

// Encrypt/Decrypt state for a single SRTP SSRC
#[derive(Default)]
pub struct SrtpSsrcState {
    ssrc: u32,
    rollover_counter: u32,
    rollover_has_processed: bool,
    last_sequence_number: u16,
    replay_detector: Option<Box<dyn ReplayDetector>>,
}

// Encrypt/Decrypt state for a single SRTCP SSRC
#[derive(Default)]
pub struct SrtcpSsrcState {
    srtcp_index: u32,
    ssrc: u32,
    replay_detector: Option<Box<dyn ReplayDetector>>,
}

impl SrtpSsrcState {
    pub fn next_rollover_count(&self, sequence_number: u16) -> u32 {
        let mut roc = self.rollover_counter;

        if !self.rollover_has_processed {
        } else if sequence_number == 0 {
            // We exactly hit the rollover count

            // Only update rolloverCounter if lastSequenceNumber is greater then MAX_ROCDISORDER
            // otherwise we already incremented for disorder
            if self.last_sequence_number > MAX_ROC_DISORDER {
                roc += 1;
            }
        } else if self.last_sequence_number < MAX_ROC_DISORDER
            && sequence_number > (MAX_SEQUENCE_NUMBER - MAX_ROC_DISORDER)
        {
            // Our last sequence number incremented because we crossed 0, but then our current number was within MAX_ROCDISORDER of the max
            // So we fell behind, drop to account for jitter
            roc -= 1;
        } else if sequence_number < MAX_ROC_DISORDER
            && self.last_sequence_number > (MAX_SEQUENCE_NUMBER - MAX_ROC_DISORDER)
        {
            // our current is within a MAX_ROCDISORDER of 0
            // and our last sequence number was a high sequence number, increment to account for jitter
            roc += 1;
        }

        roc
    }

    // https://tools.ietf.org/html/rfc3550#appendix-A.1
    pub fn update_rollover_count(&mut self, sequence_number: u16) {
        if !self.rollover_has_processed {
            self.rollover_has_processed = true;
        } else if sequence_number == 0 {
            // We exactly hit the rollover count

            // Only update rolloverCounter if lastSequenceNumber is greater then MAX_ROCDISORDER
            // otherwise we already incremented for disorder
            if self.last_sequence_number > MAX_ROC_DISORDER {
                self.rollover_counter += 1;
            }
        } else if self.last_sequence_number < MAX_ROC_DISORDER
            && sequence_number > (MAX_SEQUENCE_NUMBER - MAX_ROC_DISORDER)
        {
            // Our last sequence number incremented because we crossed 0, but then our current number was within MAX_ROCDISORDER of the max
            // So we fell behind, drop to account for jitter
            self.rollover_counter -= 1;
        } else if sequence_number < MAX_ROC_DISORDER
            && self.last_sequence_number > (MAX_SEQUENCE_NUMBER - MAX_ROC_DISORDER)
        {
            // our current is within a MAX_ROCDISORDER of 0
            // and our last sequence number was a high sequence number, increment to account for jitter
            self.rollover_counter += 1;
        }
        self.last_sequence_number = sequence_number;
    }
}

// Context represents a SRTP cryptographic context
// Context can only be used for one-way operations
// it must either used ONLY for encryption or ONLY for decryption
pub struct Context {
    cipher: Box<dyn Cipher + Send>,

    srtp_ssrc_states: HashMap<u32, SrtpSsrcState>,
    srtcp_ssrc_states: HashMap<u32, SrtcpSsrcState>,

    new_srtp_replay_detector: ContextOption,
    new_srtcp_replay_detector: ContextOption,
}

unsafe impl Send for Context {}

impl Context {
    // CreateContext creates a new SRTP Context
    pub fn new(
        master_key: &[u8],
        master_salt: &[u8],
        profile: ProtectionProfile,
        srtp_ctx_opt: Option<ContextOption>,
        srtcp_ctx_opt: Option<ContextOption>,
    ) -> Result<Context, Error> {
        let key_len = profile.key_len()?;
        let salt_len = profile.salt_len()?;

        if master_key.len() != key_len {
            return Err(Error::new(format!(
                "SRTP Master Key must be len {}, got {}",
                key_len,
                master_key.len()
            )));
        } else if master_salt.len() != salt_len {
            return Err(Error::new(format!(
                "SRTP Salt must be len {}, got {}",
                salt_len,
                master_salt.len()
            )));
        }

        let cipher: Box<dyn Cipher + Send> = Box::new(match &profile {
            &PROTECTION_PROFILE_AES128CM_HMAC_SHA1_80 => {
                CipherAesCmHmacSha1::new(master_key, master_salt)?
            }
            //&PROTECTION_PROFILE_AEAD_AES128_GCM =>CipherAeadAesGcm::new(master_key, master_salt)?,
            p => return Err(Error::new(format!("Not supported SRTP Profile {:?}", p))),
        });

        let srtp_ctx_opt = if let Some(ctx_opt) = srtp_ctx_opt {
            ctx_opt
        } else {
            srtp_no_replay_protection()
        };
        let srtcp_ctx_opt = if let Some(ctx_opt) = srtcp_ctx_opt {
            ctx_opt
        } else {
            srtcp_no_replay_protection()
        };

        Ok(Context {
            cipher,
            srtp_ssrc_states: HashMap::new(),
            srtcp_ssrc_states: HashMap::new(),
            new_srtp_replay_detector: srtp_ctx_opt,
            new_srtcp_replay_detector: srtcp_ctx_opt,
        })
    }

    fn get_srtp_ssrc_state(&mut self, ssrc: u32) -> Option<&mut SrtpSsrcState> {
        if !self.srtp_ssrc_states.contains_key(&ssrc) {
            let s = SrtpSsrcState {
                ssrc,
                replay_detector: Some((self.new_srtp_replay_detector)()),
                ..Default::default()
            };
            self.srtp_ssrc_states.insert(ssrc, s);
        }
        self.srtp_ssrc_states.get_mut(&ssrc)
    }

    fn get_srtcp_ssrc_state(&mut self, ssrc: u32) -> Option<&mut SrtcpSsrcState> {
        if !self.srtcp_ssrc_states.contains_key(&ssrc) {
            let s = SrtcpSsrcState {
                ssrc,
                replay_detector: Some((self.new_srtcp_replay_detector)()),
                ..Default::default()
            };
            self.srtcp_ssrc_states.insert(ssrc, s);
        }
        self.srtcp_ssrc_states.get_mut(&ssrc)
    }

    // roc returns SRTP rollover counter value of specified SSRC.
    fn get_roc(&self, ssrc: u32) -> Option<u32> {
        if let Some(s) = self.srtp_ssrc_states.get(&ssrc) {
            Some(s.rollover_counter)
        } else {
            None
        }
    }

    // set_roc sets SRTP rollover counter value of specified SSRC.
    fn set_roc(&mut self, ssrc: u32, roc: u32) {
        if let Some(s) = self.get_srtp_ssrc_state(ssrc) {
            s.rollover_counter = roc;
        }
    }

    // index returns SRTCP index value of specified SSRC.
    fn get_index(&self, ssrc: u32) -> Option<u32> {
        if let Some(s) = self.srtcp_ssrc_states.get(&ssrc) {
            Some(s.srtcp_index)
        } else {
            None
        }
    }

    // set_index sets SRTCP index value of specified SSRC.
    fn set_index(&mut self, ssrc: u32, index: u32) {
        if let Some(s) = self.get_srtcp_ssrc_state(ssrc) {
            s.srtcp_index = index;
        }
    }
}
