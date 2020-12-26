use bytes::BytesMut;
use util::Error;

use super::errors::*;
use super::packet::Packet;
use super::source_description::SDESType;
use crate::receiver_report::ReceiverReport;
use crate::sender_report::SenderReport;

mod compound_packet_test;

/// A CompoundPacket is a collection of RTCP packets transmitted as a single packet with
/// the underlying protocol (for example UDP).
///
/// To maximize the resolution of receiption statistics, the first Packet in a CompoundPacket
/// must always be either a SenderReport or a ReceiverReport.  This is true even if no data
/// has been sent or received, in which case an empty ReceiverReport must be sent, and even
/// if the only other RTCP packet in the compound packet is a Goodbye.
///
/// Next, a SourceDescription containing a CNAME item must be included in each CompoundPacket
/// to identify the source and to begin associating media for purposes such as lip-sync.
///
/// Other RTCP packet types may follow in any order. Packet types may appear more than once.
#[derive(Default)]
pub struct CompoundPacket(Vec<Box<dyn Packet>>);

impl CompoundPacket {
    /// Validate returns an error if this is not an RFC-compliant CompoundPacket.
    pub fn validate(&self) -> Result<(), Error> {
        if self.0.is_empty() {
            return Err(ERR_EMPTY_COMPOUND.clone());
        }

        // ToDo: Any way to match types cleanly???? @metaclips
        // ToDo: We need proper error handling. @metaclips
        // SenderReport and ReceiverReport are the only types that
        // are allowed to be the first packet in a compound datagram
        if self.0[0].as_any().downcast_ref::<SenderReport>().is_none()
            && self.0[0]
                .as_any()
                .downcast_ref::<ReceiverReport>()
                .is_none()
        {
            return Err(ERR_BAD_FIRST_PACKET.clone());
        }

        for pkt in &self.0[1..] {
            // If the number of RecetpionReports exceeds 31 additional ReceiverReports
            // can be included here.
            if let Some(_) = pkt.as_any().downcast_ref::<ReceiverReport>() {
                continue;
            // A SourceDescription containing a CNAME must be included in every
            // CompoundPacket.
            } else if let Some(e) = pkt
                .as_any()
                .downcast_ref::<crate::source_description::SourceDescription>()
            {
                let e: &crate::source_description::SourceDescription = e;

                let mut has_cname = false;
                for c in &e.chunks {
                    for it in &c.items {
                        if it.sdes_type == SDESType::SDESCNAME {
                            has_cname = true
                        }
                    }

                    if !has_cname {
                        return Err(ERR_MISSING_CNAME.clone());
                    }

                    return Ok(());
                }
            // Other packets are not permitted before the CNAME
            } else {
                return Err(ERR_PACKET_BEFORE_CNAME.clone());
            }
        }

        // CNAME never reached
        Err(ERR_MISSING_CNAME.clone())
    }

    /// CNAME returns the CNAME that *must* be present in every CompoundPacket
    pub fn cname(&self) -> Result<String, Error> {
        if self.0.is_empty() {
            return Err(ERR_EMPTY_COMPOUND.clone());
        }

        for pkt in &self.0[1..] {
            if let Some(sdes) = pkt
                .as_any()
                .downcast_ref::<crate::source_description::SourceDescription>()
            {
                let sdes: &crate::source_description::SourceDescription = sdes;

                for c in &sdes.chunks {
                    for it in &c.items {
                        if it.sdes_type == SDESType::SDESCNAME {
                            return Ok(it.text.to_owned());
                        }
                    }
                }
            } else if let None = pkt
                .as_any()
                .downcast_ref::<crate::receiver_report::ReceiverReport>()
            {
                return Err(ERR_PACKET_BEFORE_CNAME.to_owned());
            }
        }

        Err(ERR_MISSING_CNAME.clone())
    }

    /// Marshal encodes the CompoundPacket as binary.
    pub fn marshal(&self) -> Result<BytesMut, Error> {
        self.validate()?;

        crate::packet::marshal(&self.0)
    }

    pub fn unmarshal(&mut self, mut raw_data: BytesMut) -> Result<(), Error> {
        let mut out = Vec::new();

        while raw_data.len() != 0 {
            let (p, processed) = crate::packet::unmarshaller(&mut raw_data)?;
            out.push(p);

            raw_data = raw_data.split_off(processed);
        }

        *self = Self(out);

        self.validate()
    }

    /// destination_ssrc returns the synchronization sources associated with this
    /// CompoundPacket's reception report.
    pub fn destination_ssrc(&self) -> Vec<u32> {
        if self.0.is_empty() {
            vec![]
        } else {
            self.0[0].destination_ssrc()
        }
    }
}
