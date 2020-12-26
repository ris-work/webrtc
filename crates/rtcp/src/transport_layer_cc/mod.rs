//! https://tools.ietf.org/html/draft-holmer-rmcat-transport-wide-cc-extensions-01#page-5
//! 0                   1                   2                   3
//! 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |V=2|P|  FMT=15 |    PT=205     |           length              |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                     SSRC of packet sender                     |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                      SSRC of media source                     |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |      base sequence number     |      packet status count      |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                 reference time                | fb pkt. count |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |          packet chunk         |         packet chunk          |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! .                                                               .
//! .                                                               .
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |         packet chunk          |  recv delta   |  recv delta   |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! .                                                               .
//! .                                                               .
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |           recv delta          |  recv delta   | zero padding  |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

mod transport_layer_cc_test;

use std::fmt;

use byteorder::{BigEndian, ByteOrder};

use bytes::BytesMut;
use util::Error;
use utility::get_nbits_from_byte;

use crate::{packet::Packet, receiver_report};

use super::header;
use super::util as utility;
use crate::errors::*;

// for packet status chunk
/// type of packet status chunk
#[derive(PartialEq, Debug, Clone)]
#[repr(u16)]
pub enum StatusChunkTypeTCC {
    RunLengthChunk = 0,
    StatusVectorChunk = 1,
}

/// type of packet status symbol and recv delta
#[derive(PartialEq, Debug, Clone)]
#[repr(u16)]
pub enum SymbolTypeTCC {
    /// https://tools.ietf.org/html/draft-holmer-rmcat-transport-wide-cc-extensions-01#section-3.1.1
    PacketNotReceived = 0,
    /// https://tools.ietf.org/html/draft-holmer-rmcat-transport-wide-cc-extensions-01#section-3.1.1
    PacketReceivedSmallDelta = 1,
    /// https://tools.ietf.org/html/draft-holmer-rmcat-transport-wide-cc-extensions-01#section-3.1.1
    PacketReceivedLargeDelta = 2,
    /// https://tools.ietf.org/html/draft-holmer-rmcat-transport-wide-cc-extensions-01#page-7
    /// see Example 2: "packet received, w/o recv delta"
    PacketReceivedWithoutDelta = 3,
}

/// for status vector chunk
#[derive(PartialEq, Debug, Clone)]
#[repr(u16)]
pub enum SymbolSizeTypeTCC {
    /// https://tools.ietf.org/html/draft-holmer-rmcat-transport-wide-cc-extensions-01#section-3.1.4
    OneBit = 0,
    TwoBit = 1,
}

impl From<u16> for SymbolSizeTypeTCC {
    fn from(val: u16) -> Self {
        match val {
            0 => SymbolSizeTypeTCC::OneBit,
            _ => SymbolSizeTypeTCC::TwoBit,
        }
    }
}

impl Default for SymbolSizeTypeTCC {
    fn default() -> Self {
        SymbolSizeTypeTCC::OneBit
    }
}

impl From<u16> for StatusChunkTypeTCC {
    fn from(val: u16) -> Self {
        match val {
            0 => StatusChunkTypeTCC::RunLengthChunk,
            _ => StatusChunkTypeTCC::StatusVectorChunk,
        }
    }
}

impl Default for StatusChunkTypeTCC {
    fn default() -> Self {
        StatusChunkTypeTCC::RunLengthChunk
    }
}

impl From<u16> for SymbolTypeTCC {
    fn from(val: u16) -> Self {
        match val {
            0 => SymbolTypeTCC::PacketNotReceived,
            1 => SymbolTypeTCC::PacketReceivedSmallDelta,
            2 => SymbolTypeTCC::PacketReceivedLargeDelta,
            _ => SymbolTypeTCC::PacketReceivedWithoutDelta,
        }
    }
}

impl Default for SymbolTypeTCC {
    fn default() -> Self {
        SymbolTypeTCC::PacketNotReceived
    }
}

/// PacketStatusChunk has two kinds:
/// RunLengthChunk and StatusVectorChunk
pub trait PacketStatusChunk {
    fn as_any(&self) -> &dyn std::any::Any;
    fn trait_eq(&self, other: &dyn PacketStatusChunk) -> bool;
    fn marshal(&self) -> Result<BytesMut, Error>;
    fn unmarshal(&mut self, raw_packet: &mut BytesMut) -> Result<(), Error>;
}

impl PartialEq for dyn PacketStatusChunk {
    fn eq(&self, other: &dyn PacketStatusChunk) -> bool {
        self.trait_eq(other)
    }
}

/// RunLengthChunk T=TypeTCCRunLengthChunk
/// 0                   1
/// 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |T| S |       Run Length        |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
#[derive(Debug, Clone, PartialEq, Default)]
struct RunLengthChunk {
    /// T = TypeTCCRunLengthChunk
    type_tcc: StatusChunkTypeTCC,
    /// S: type of packet status
    /// kind: TypeTCCPacketNotReceived or...
    packet_status_symbol: SymbolTypeTCC,
    /// run_length: count of S
    run_length: u16,
}

impl PacketStatusChunk for RunLengthChunk {
    /// Marshal ..
    fn marshal(&self) -> Result<BytesMut, Error> {
        let mut chunks = vec![0u8; 2];

        // append 1 bit '0'
        let mut dst = utility::set_nbits_of_uint16(0, 1, 0, 0)?;

        // append 2 bit packet_status_symbol
        dst = utility::set_nbits_of_uint16(dst, 2, 1, self.packet_status_symbol.clone() as u16)?;

        // append 13 bit run_length
        dst = utility::set_nbits_of_uint16(dst, 13, 3, self.run_length)?;

        BigEndian::write_u16(&mut chunks, dst);

        Ok(chunks[..].into())
    }

    /// Unmarshal ..
    fn unmarshal(&mut self, raw_packet: &mut BytesMut) -> Result<(), Error> {
        if raw_packet.len() != PACKET_STATUS_CHUNK_LENGTH as usize {
            return Err(Error::new(
                "packet status chunk must be 2 bytes".to_string(),
            ));
        }

        // record type
        self.type_tcc = StatusChunkTypeTCC::RunLengthChunk;

        // get PacketStatusSymbol
        // r.PacketStatusSymbol = uint16(rawPacket[0] >> 5 & 0x03)
        self.packet_status_symbol =
            (utility::get_nbits_from_byte(raw_packet[0], 1, 2) as u16).into();

        // get RunLength
        // r.RunLength = uint16(rawPacket[0]&0x1F)*256 + uint16(rawPacket[1])
        self.run_length = ((utility::get_nbits_from_byte(raw_packet[0], 3, 5) as usize) << 8)
            as u16
            + (raw_packet[1] as u16);

        Ok(())
    }

    fn trait_eq(&self, other: &dyn PacketStatusChunk) -> bool {
        other
            .as_any()
            .downcast_ref::<RunLengthChunk>()
            .map_or(false, |a| self == a)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// StatusVectorChunk T=typeStatusVecotrChunk
/// 0                   1
/// 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |T|S|       symbol list         |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
#[derive(Debug, Clone, PartialEq, Default)]
struct StatusVectorChunk {
    // T = TypeTCCRunLengthChunk
    type_tcc: StatusChunkTypeTCC,

    // TypeTCCSymbolSizeOneBit or TypeTCCSymbolSizeTwoBit
    symbol_size: SymbolSizeTypeTCC,

    // when symbol_size = TypeTCCSymbolSizeOneBit, symbol_list is 14*1bit:
    // TypeTCCSymbolListPacketReceived or TypeTCCSymbolListPacketNotReceived
    // when symbol_size = TypeTCCSymbolSizeTwoBit, symbol_list is 7*2bit:
    // TypeTCCPacketNotReceived TypeTCCPacketReceivedSmallDelta TypeTCCPacketReceivedLargeDelta or typePacketReserved
    symbol_list: Vec<SymbolTypeTCC>,
}

impl PacketStatusChunk for StatusVectorChunk {
    // Marshal ..
    fn marshal(&self) -> Result<BytesMut, Error> {
        let mut chunk = vec![0u8; 2];

        // set first bit '1'
        let mut dst = utility::set_nbits_of_uint16(0, 1, 0, 1)?;

        // set second bit symbol_size
        dst = utility::set_nbits_of_uint16(dst, 1, 1, self.symbol_size.clone() as u16)?;

        let num_of_bits = NUM_OF_BITS_OF_SYMBOL_SIZE[self.symbol_size.clone() as usize];
        // append 14 bit symbol_list
        for (i, s) in self.symbol_list.iter().enumerate() {
            let index = num_of_bits * (i as u16) + 2;
            dst = utility::set_nbits_of_uint16(dst, num_of_bits, index, s.clone() as u16)?;
        }

        BigEndian::write_u16(&mut chunk, dst);

        // set SymbolList(bit8-15)
        // chunk[1] = uint8(r.SymbolList) & 0x0f
        Ok(chunk[..].into())
    }

    // Unmarshal ..
    fn unmarshal(&mut self, raw_packet: &mut BytesMut) -> Result<(), Error> {
        if raw_packet.len() != PACKET_STATUS_CHUNK_LENGTH {
            return Err(ERR_PACKET_BEFORE_CNAME.to_owned());
        }

        self.type_tcc = StatusChunkTypeTCC::StatusVectorChunk;
        self.symbol_size = get_nbits_from_byte(raw_packet[0], 1, 1).into();

        match self.symbol_size {
            SymbolSizeTypeTCC::OneBit => {
                for i in 0..6u16 {
                    self.symbol_list
                        .push(get_nbits_from_byte(raw_packet[0], 2 + i, 1).into());
                }

                for i in 0..8u16 {
                    self.symbol_list
                        .push(get_nbits_from_byte(raw_packet[1], i, 1).into())
                }

                Ok(())
            }

            SymbolSizeTypeTCC::TwoBit => {
                for i in 0..3u16 {
                    self.symbol_list
                        .push(get_nbits_from_byte(raw_packet[0], 2 + i * 2, 2).into());
                }

                for i in 0..4u16 {
                    self.symbol_list
                        .push(get_nbits_from_byte(raw_packet[1], i * 2, 2).into());
                }

                Ok(())
            }
        }
    }

    fn trait_eq(&self, other: &dyn PacketStatusChunk) -> bool {
        other
            .as_any()
            .downcast_ref::<StatusVectorChunk>()
            .map_or(false, |a| self == a)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// RecvDelta are represented as multiples of 250us
/// small delta is 1 byte: [0，63.75]ms = [0, 63750]us = [0, 255]*250us
/// big delta is 2 bytes: [-8192.0, 8191.75]ms = [-8192000, 8191750]us = [-32768, 32767]*250us
/// https://tools.ietf.org/html/draft-holmer-rmcat-transport-wide-cc-extensions-01#section-3.1.5
#[derive(Debug, Clone, PartialEq, Default)]
struct RecvDelta {
    type_tcc_packet: SymbolTypeTCC,
    /// us
    delta: i64,
}

impl RecvDelta {
    /// Marshal ..
    pub fn marshal(&self) -> Result<BytesMut, Error> {
        let delta = self.delta / TYPE_TCC_DELTA_SCALE_FACTOR;

        // small delta
        if self.type_tcc_packet == SymbolTypeTCC::PacketReceivedSmallDelta
            && delta >= 0
            && delta <= std::u8::MAX as i64
        {
            let mut delta_chunk = vec![0u8; 1];
            delta_chunk[0] = delta as u8;
            return Ok(delta_chunk[..].into());
        }

        // big delta
        if self.type_tcc_packet == SymbolTypeTCC::PacketReceivedLargeDelta
            && delta >= std::i16::MIN as i64
            && delta <= std::u16::MAX as i64
        {
            let mut delta_chunk = vec![0u8; 2];
            BigEndian::write_u16(&mut delta_chunk, delta as u16);
            return Ok(delta_chunk[..].into());
        }

        // overflow
        Err(ERR_DELTA_EXCEED_LIMIT.to_owned())
    }

    /// Unmarshal ..
    pub fn unmarshal(&mut self, raw_packet: &mut BytesMut) -> Result<(), Error> {
        let chunk_len = raw_packet.len();

        // must be 1 or 2 bytes
        if chunk_len != 1 && chunk_len != 2 {
            return Err(ERR_DELTA_EXCEED_LIMIT.to_owned());
        }

        if chunk_len == 1 {
            self.type_tcc_packet = SymbolTypeTCC::PacketReceivedSmallDelta;
            self.delta = TYPE_TCC_DELTA_SCALE_FACTOR * raw_packet[0] as i64;
            return Ok(());
        }

        self.type_tcc_packet = SymbolTypeTCC::PacketReceivedLargeDelta;
        self.delta = TYPE_TCC_DELTA_SCALE_FACTOR * BigEndian::read_i16(raw_packet) as i64;

        Ok(())
    }
}

/// The offset after header
const BASE_SEQUENCE_NUMBER_OFFSET: usize = 8;
/// The offset after header
const PACKET_STATUS_COUNT_OFFSET: usize = 10;
/// The offset after header
const REFERENCE_TIME_OFFSET: usize = 12;
/// The offset after header
const FB_PKT_COUNT_OFFSET: usize = 15;
/// The offset after header
const PACKET_CHUNK_OFFSET: usize = 16;
/// len of packet status chunk
const TYPE_TCC_STATUS_VECTOR_CHUNK: usize = 1;

/// https://tools.ietf.org/html/draft-holmer-rmcat-transport-wide-cc-extensions-01#section-3.1.5
const TYPE_TCC_DELTA_SCALE_FACTOR: i64 = 250;

// Notice: RFC is wrong: "packet received" (0) and "packet not received" (1)
// if S == TYPE_TCCSYMBOL_SIZE_ONE_BIT, symbol list will be: TypeTCCPacketNotReceived TypeTCCPacketReceivedSmallDelta
// if S == TYPE_TCCSYMBOL_SIZE_TWO_BIT, symbol list will be same as above:

static NUM_OF_BITS_OF_SYMBOL_SIZE: [u16; 2] = [1, 2];

/// len of packet status chunk
const PACKET_STATUS_CHUNK_LENGTH: usize = 2;

/// TransportLayerCC for sender-BWE
/// https://tools.ietf.org/html/draft-holmer-rmcat-transport-wide-cc-extensions-01#page-5
#[derive(Default, PartialEq)]
pub struct TransportLayerCC {
    /// header
    header: header::Header,
    /// SSRC of sender
    sender_ssrc: u32,
    /// SSRC of the media source
    media_ssrc: u32,
    /// Transport wide sequence of rtp extension
    base_sequence_number: u16,
    /// packet_status_count
    packet_status_count: u16,
    /// reference_time
    reference_time: u32,
    /// fb_pkt_count
    fb_pkt_count: u8,
    /// packet_chunks
    packet_chunks: Vec<Box<dyn PacketStatusChunk>>,
    /// recv_deltas
    recv_deltas: Vec<RecvDelta>,
}

impl fmt::Display for TransportLayerCC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = format!("TransportLayerCC:\n\tHeader {:?}\n", self.header);
        out += format!("TransportLayerCC:\n\tSender Ssrc {}\n", self.sender_ssrc).as_str();
        out += format!("\tMedia Ssrc {}\n", self.media_ssrc).as_str();
        out += format!("\tBase Sequence Number {}\n", self.base_sequence_number).as_str();
        out += format!("\tStatus Count {}\n", self.packet_status_count).as_str();
        out += format!("\tReference Time {}\n", self.reference_time).as_str();
        out += format!("\tFeedback Packet Count {}\n", self.fb_pkt_count).as_str();
        out += "\tpacket_chunks ";
        out += "\n\trecv_deltas ";
        for delta in &self.recv_deltas {
            out += format!("{:?} ", delta).as_str();
        }
        out += "\n";

        write!(f, "{}", out)
    }
}

impl Packet for TransportLayerCC {
    /// Unmarshal ..
    fn unmarshal(&mut self, raw_packet: &mut BytesMut) -> Result<(), Error> {
        if raw_packet.len() < (header::HEADER_LENGTH + receiver_report::SSRC_LENGTH) {
            return Err(ERR_PACKET_TOO_SHORT.to_owned());
        }

        if let Err(e) = self.header.unmarshal(raw_packet) {
            return Err(e);
        }
        // https://tools.ietf.org/html/rfc4585#page-33
        // header's length + payload's length
        let total_length = 4 * (self.header.length + 1);

        if total_length as usize <= header::HEADER_LENGTH + PACKET_CHUNK_OFFSET {
            return Err(ERR_PACKET_TOO_SHORT.to_owned());
        }

        if raw_packet.len() < total_length as usize {
            return Err(ERR_PACKET_TOO_SHORT.to_owned());
        }

        if self.header.packet_type != header::PacketType::TransportSpecificFeedback
            || self.header.count != header::FORMAT_TCC
        {
            return Err(ERR_WRONG_TYPE.to_owned());
        }

        self.sender_ssrc = BigEndian::read_u32(&raw_packet[header::HEADER_LENGTH..]);
        self.media_ssrc = BigEndian::read_u32(
            &raw_packet[header::HEADER_LENGTH + receiver_report::SSRC_LENGTH..],
        );
        self.base_sequence_number =
            BigEndian::read_u16(&raw_packet[header::HEADER_LENGTH + BASE_SEQUENCE_NUMBER_OFFSET..]);

        self.packet_status_count =
            BigEndian::read_u16(&raw_packet[header::HEADER_LENGTH + PACKET_STATUS_COUNT_OFFSET..]);

        self.reference_time = utility::get_24bits_from_bytes(
            &raw_packet[header::HEADER_LENGTH + REFERENCE_TIME_OFFSET
                ..header::HEADER_LENGTH + REFERENCE_TIME_OFFSET + 3],
        );

        self.fb_pkt_count = raw_packet[header::HEADER_LENGTH + FB_PKT_COUNT_OFFSET];

        let mut packet_status_pos = header::HEADER_LENGTH + PACKET_CHUNK_OFFSET;

        let mut processed_packet_num = 0u16;

        while processed_packet_num < self.packet_status_count {
            if packet_status_pos + PACKET_STATUS_CHUNK_LENGTH >= total_length as usize {
                return Err(ERR_PACKET_TOO_SHORT.to_owned());
            }

            let typ = utility::get_nbits_from_byte(
                (raw_packet[packet_status_pos..packet_status_pos + 1])[0],
                0,
                1,
            );

            let initial_packet_status: Box<dyn PacketStatusChunk>;

            match typ.into() {
                StatusChunkTypeTCC::RunLengthChunk => {
                    let mut packet_status = RunLengthChunk {
                        type_tcc: typ.into(),
                        ..Default::default()
                    };

                    packet_status.unmarshal(
                        &mut raw_packet[packet_status_pos..packet_status_pos + 2].into(),
                    )?;

                    let packet_number_to_process = (self.packet_status_count
                        - processed_packet_num)
                        .min(packet_status.run_length);

                    if packet_status.packet_status_symbol == SymbolTypeTCC::PacketReceivedSmallDelta
                        || packet_status.packet_status_symbol
                            == SymbolTypeTCC::PacketReceivedLargeDelta
                    {
                        let mut j = 0u16;

                        while j < packet_number_to_process {
                            self.recv_deltas.push(RecvDelta {
                                type_tcc_packet: packet_status.packet_status_symbol.clone(),
                                ..Default::default()
                            });

                            j += 1;
                        }
                    }

                    initial_packet_status = Box::new(packet_status);
                    processed_packet_num += packet_number_to_process;
                }

                StatusChunkTypeTCC::StatusVectorChunk => {
                    let mut packet_status = StatusVectorChunk {
                        type_tcc: typ.into(),
                        ..Default::default()
                    };

                    packet_status.unmarshal(
                        &mut raw_packet[packet_status_pos..packet_status_pos + 2].into(),
                    )?;

                    match packet_status.symbol_size {
                        SymbolSizeTypeTCC::OneBit => {
                            for sym in &packet_status.symbol_list {
                                if *sym == SymbolTypeTCC::PacketReceivedSmallDelta {
                                    self.recv_deltas.push(RecvDelta {
                                        type_tcc_packet: SymbolTypeTCC::PacketReceivedSmallDelta,
                                        ..Default::default()
                                    })
                                }
                            }
                        }

                        SymbolSizeTypeTCC::TwoBit => {
                            for sym in &packet_status.symbol_list {
                                if *sym == SymbolTypeTCC::PacketReceivedSmallDelta
                                    || *sym == SymbolTypeTCC::PacketReceivedLargeDelta
                                {
                                    self.recv_deltas.push(RecvDelta {
                                        type_tcc_packet: sym.clone(),
                                        ..Default::default()
                                    })
                                }
                            }
                        }
                    }

                    processed_packet_num += packet_status.symbol_list.len() as u16;
                    initial_packet_status = Box::new(packet_status);
                }
            }

            packet_status_pos += PACKET_STATUS_CHUNK_LENGTH;
            self.packet_chunks.push(initial_packet_status);
        }

        let mut recv_deltas_pos = packet_status_pos;

        for delta in &mut self.recv_deltas {
            if recv_deltas_pos >= total_length as usize {
                return Err(ERR_PACKET_TOO_SHORT.to_owned());
            }

            if delta.type_tcc_packet == SymbolTypeTCC::PacketReceivedSmallDelta {
                delta.unmarshal(&mut raw_packet[recv_deltas_pos..recv_deltas_pos + 1].into())?;

                recv_deltas_pos += 1;
            }

            if delta.type_tcc_packet == SymbolTypeTCC::PacketReceivedLargeDelta {
                delta.unmarshal(&mut raw_packet[recv_deltas_pos..recv_deltas_pos + 2].into())?;

                recv_deltas_pos += 2;
            }
        }

        Ok(())
    }

    fn marshal(&self) -> Result<BytesMut, Error> {
        let mut header = self.header.marshal()?;

        let mut payload = BytesMut::new();
        payload.resize(self.len() - header::HEADER_LENGTH, 0u8);

        BigEndian::write_u32(&mut payload, self.sender_ssrc);
        BigEndian::write_u32(&mut payload[4..], self.media_ssrc);
        BigEndian::write_u16(
            &mut payload[BASE_SEQUENCE_NUMBER_OFFSET..],
            self.base_sequence_number,
        );
        BigEndian::write_u16(
            &mut payload[PACKET_STATUS_COUNT_OFFSET..],
            self.packet_status_count,
        );

        let reference_time_and_fb_pkt_count =
            utility::append_nbits_to_uint32(0, 24, self.reference_time);
        let reference_time_and_fb_pkt_count = utility::append_nbits_to_uint32(
            reference_time_and_fb_pkt_count,
            8,
            self.fb_pkt_count as u32,
        );

        BigEndian::write_u32(
            &mut payload[REFERENCE_TIME_OFFSET..],
            reference_time_and_fb_pkt_count,
        );

        for (i, chunk) in self.packet_chunks.iter().enumerate() {
            let b = chunk.marshal()?;

            let v = PACKET_CHUNK_OFFSET + i * 2;
            payload[v..v + b.len()].copy_from_slice(&b);
        }

        let recv_delta_offset = PACKET_CHUNK_OFFSET + self.packet_chunks.len() * 2;
        let mut i = 0usize;

        for delta in &self.recv_deltas {
            let b = delta.marshal()?;

            payload[recv_delta_offset + i..b.len() + recv_delta_offset + i].copy_from_slice(&b);
            i += 1;

            if delta.type_tcc_packet == SymbolTypeTCC::PacketReceivedLargeDelta {
                i += 1;
            }
        }

        if self.header.padding {
            let len = payload.len();

            payload[len - 1] = (self.len() - self.packet_len()) as u8;
        }

        header.extend(&payload);

        Ok(header)
    }

    /// destination_ssrc returns an array of SSRC values that this packet refers to.
    fn destination_ssrc(&self) -> Vec<u32> {
        vec![self.media_ssrc]
    }

    fn trait_eq(&self, other: &dyn Packet) -> bool {
        other
            .as_any()
            .downcast_ref::<TransportLayerCC>()
            .map_or(false, |a| self == a)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl TransportLayerCC {
    fn packet_len(&self) -> usize {
        let mut n = header::HEADER_LENGTH + PACKET_CHUNK_OFFSET + self.packet_chunks.len() * 2;
        for d in &self.recv_deltas {
            let delta = d.delta / TYPE_TCC_DELTA_SCALE_FACTOR;

            // small delta
            if d.type_tcc_packet == SymbolTypeTCC::PacketReceivedSmallDelta
                && delta >= 0
                && delta <= u8::MAX as i64
            {
                n += 1;
            }

            if d.type_tcc_packet == SymbolTypeTCC::PacketReceivedLargeDelta
                && delta >= i16::MIN as i64
                && delta <= i16::MAX as i64
            {
                n += 2
            }
        }
        n
    }

    fn len(&self) -> usize {
        let n = self.packet_len();
        n + utility::get_padding(n)
    }
}
