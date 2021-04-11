use crate::error::Error;
use crate::packetizer::Payloader;

use bytes::Bytes;

#[cfg(test)]
mod g7xx_test;

/// G711Payloader payloads G711 packets
pub type G711Payloader = G7xxPayloader;
/// G722Payloader payloads G722 packets
pub type G722Payloader = G7xxPayloader;

pub struct G7xxPayloader;

impl Payloader for G7xxPayloader {
    /// Payload fragments an G7xx packet across one or more byte arrays
    fn payload(&self, mtu: usize, payload: &Bytes) -> Result<Vec<Bytes>, Error> {
        if payload.is_empty() || mtu == 0 {
            return Ok(vec![]);
        }

        let mut payloads = vec![];
        let mut payload_data_remaining = payload.len();
        let mut payload_data_index = 0;
        while payload_data_remaining > 0 {
            let current_fragment_size = std::cmp::min(mtu as usize, payload_data_remaining);
            payloads.push(
                payload.slice(payload_data_index..payload_data_index + current_fragment_size),
            );

            payload_data_remaining -= current_fragment_size;
            payload_data_index += current_fragment_size;
        }

        Ok(payloads)
    }
}
