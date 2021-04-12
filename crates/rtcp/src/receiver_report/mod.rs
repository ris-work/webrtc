#[cfg(test)]
mod receiver_report_test;

use crate::{error::Error, header::*, packet::*, reception_report::*, util::*};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::fmt;

pub(super) const RR_SSRC_OFFSET: usize = HEADER_LENGTH;
pub(super) const RR_REPORT_OFFSET: usize = RR_SSRC_OFFSET + SSRC_LENGTH;

/// A ReceiverReport (RR) packet provides reception quality feedback for an RTP stream
#[derive(Debug, PartialEq, Default, Clone)]
pub struct ReceiverReport {
    /// The synchronization source identifier for the originator of this RR packet.
    pub ssrc: u32,
    /// Zero or more reception report blocks depending on the number of other
    /// sources heard by this sender since the last report. Each reception report
    /// block conveys statistics on the reception of RTP packets from a
    /// single synchronization source.
    pub reports: Vec<ReceptionReport>,
    /// Extension contains additional, payload-specific information that needs to
    /// be reported regularly about the receiver.
    pub profile_extensions: Bytes,
}

impl fmt::Display for ReceiverReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = format!("ReceiverReport from {}\n", self.ssrc);
        out += "\tSSRC    \tLost\tLastSequence\n";
        for rep in &self.reports {
            out += format!(
                "\t{:x}\t{}/{}\t{}\n",
                rep.ssrc, rep.fraction_lost, rep.total_lost, rep.last_sequence_number
            )
            .as_str();
        }
        out += format!("\tProfile Extension Data: {:?}\n", self.profile_extensions).as_str();

        write!(f, "{}", out)
    }
}

impl Packet for ReceiverReport {
    /// destination_ssrc returns an array of SSRC values that this packet refers to.
    fn destination_ssrc(&self) -> Vec<u32> {
        self.reports.iter().map(|x| x.ssrc).collect()
    }

    fn marshal_size(&self) -> usize {
        let mut reps_length = 0;
        for rep in &self.reports {
            reps_length += rep.marshal_size();
        }

        let l = HEADER_LENGTH + SSRC_LENGTH + reps_length + self.profile_extensions.len();

        // align to 32-bit boundary
        l + get_padding(l)
    }

    /// Marshal encodes the packet in binary.
    fn marshal(&self) -> Result<Bytes, Error> {
        if self.reports.len() > COUNT_MAX {
            return Err(Error::TooManyReports);
        }
        /*
         *         0                   1                   2                   3
         *         0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         * header |V=2|P|    RC   |   PT=RR=201   |             length            |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *        |                     SSRC of packet sender                     |
         *        +=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+
         * report |                 SSRC_1 (SSRC of first source)                 |
         * block  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *   1    | fraction lost |       cumulative number of packets lost       |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *        |           extended highest sequence number received           |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *        |                      interarrival jitter                      |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *        |                         last SR (LSR)                         |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *        |                   delay since last SR (DLSR)                  |
         *        +=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+
         * report |                 SSRC_2 (SSRC of second source)                |
         * block  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *   2    :                               ...                             :
         *        +=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+
         *        |                  profile-specific extensions                  |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         */

        let mut writer = BytesMut::with_capacity(self.marshal_size());

        let h = self.header();
        let data = h.marshal()?;
        writer.extend(data);

        writer.put_u32(self.ssrc);

        for report in &self.reports {
            let data = report.marshal()?;
            writer.extend(data);
        }

        writer.extend(self.profile_extensions.clone());

        put_padding(&mut writer);
        Ok(writer.freeze())
    }

    /// Unmarshal decodes the ReceiverReport from binary
    fn unmarshal(raw_packet: &Bytes) -> Result<Self, Error> {
        /*
         *         0                   1                   2                   3
         *         0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         * header |V=2|P|    RC   |   PT=RR=201   |             length            |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *        |                     SSRC of packet sender                     |
         *        +=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+
         * report |                 SSRC_1 (SSRC of first source)                 |
         * block  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *   1    | fraction lost |       cumulative number of packets lost       |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *        |           extended highest sequence number received           |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *        |                      interarrival jitter                      |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *        |                         last SR (LSR)                         |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *        |                   delay since last SR (DLSR)                  |
         *        +=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+
         * report |                 SSRC_2 (SSRC of second source)                |
         * block  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         *   2    :                               ...                             :
         *        +=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+
         *        |                  profile-specific extensions                  |
         *        +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
         */

        if raw_packet.len() < (HEADER_LENGTH + SSRC_LENGTH) {
            return Err(Error::PacketTooShort);
        }

        let header = Header::unmarshal(raw_packet)?;

        if header.packet_type != PacketType::ReceiverReport {
            return Err(Error::WrongType);
        }

        let reader = &mut raw_packet.slice(RR_SSRC_OFFSET..);

        let ssrc = reader.get_u32();

        let mut i = RR_REPORT_OFFSET;
        let mut reports = vec![];
        while i < raw_packet.len() && reports.len() < header.count as usize {
            let rr = ReceptionReport::unmarshal(&raw_packet.slice(i..))?;
            reports.push(rr);
            i += RECEPTION_REPORT_LENGTH;
        }

        let profile_extensions =
            raw_packet.slice(RR_REPORT_OFFSET + (reports.len() * RECEPTION_REPORT_LENGTH)..);

        if reports.len() != header.count as usize {
            return Err(Error::InvalidHeader);
        }

        Ok(ReceiverReport {
            ssrc,
            reports,
            profile_extensions,
        })
    }

    /*fn equal(&self, other: &dyn Packet) -> bool {
        other
            .as_any()
            .downcast_ref::<ReceiverReport>()
            .map_or(false, |a| self == a)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }*/
}

impl ReceiverReport {
    // Header returns the Header associated with this packet.
    pub fn header(&self) -> Header {
        Header {
            padding: false,
            count: self.reports.len() as u8,
            packet_type: PacketType::ReceiverReport,
            length: ((self.marshal_size() / 4) - 1) as u16,
        }
    }
}
