use crate::chunk::Chunk;

use crate::chunk::chunk_abort::ChunkAbort;
use crate::chunk::chunk_cookie_ack::ChunkCookieAck;
use crate::chunk::chunk_cookie_echo::ChunkCookieEcho;
use crate::chunk::chunk_error::ChunkError;
use crate::chunk::chunk_forward_tsn::ChunkForwardTsn;
use crate::chunk::chunk_header::*;
use crate::chunk::chunk_heartbeat::ChunkHeartbeat;
use crate::chunk::chunk_init::ChunkInit;
use crate::chunk::chunk_payload_data::ChunkPayloadData;
use crate::chunk::chunk_reconfig::ChunkReconfig;
use crate::chunk::chunk_selective_ack::ChunkSelectiveAck;
use crate::chunk::chunk_shutdown::ChunkShutdown;
use crate::chunk::chunk_shutdown_ack::ChunkShutdownAck;
use crate::chunk::chunk_shutdown_complete::ChunkShutdownComplete;
use crate::chunk::chunk_type::*;
use crate::error::Error;
use crate::util::*;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use crc::{crc32, Hasher32};
use std::fmt;

///Packet represents an SCTP packet, defined in https://tools.ietf.org/html/rfc4960#section-3
///An SCTP packet is composed of a common header and chunks.  A chunk
///contains either control information or user data.
///
///
///SCTP Packet Format
/// 0                   1                   2                   3
/// 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///|                        Common Header                          |
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///|                          Chunk #1                             |
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///|                           ...                                 |
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///|                          Chunk #n                             |
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///
///
///SCTP Common Header Format
///
/// 0                   1                   2                   3
/// 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///|     Source Value Number        |     Destination Value Number |
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///|                      Verification Tag                         |
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///|                           Checksum                            |
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

pub(crate) struct Packet {
    pub(crate) source_port: u16,
    pub(crate) destination_port: u16,
    pub(crate) verification_tag: u32,
    pub(crate) chunks: Vec<Box<dyn Chunk>>,
}

/// makes packet printable
impl fmt::Display for Packet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut res = format!(
            "Packet:
        source_port: {}
        destination_port: {}
        verification_tag: {}
        ",
            self.source_port, self.destination_port, self.verification_tag,
        );
        for chunk in &self.chunks {
            res += format!("Chunk: {}", chunk.to_string()).as_str();
        }
        write!(f, "{}", res)
    }
}

pub(crate) const PACKET_HEADER_SIZE: usize = 12;

impl Packet {
    pub(crate) fn unmarshal(raw: &Bytes) -> Result<Self, Error> {
        if raw.len() < PACKET_HEADER_SIZE {
            return Err(Error::ErrPacketRawTooSmall);
        }

        let reader = &mut raw.clone();

        let source_port = reader.get_u16();
        let destination_port = reader.get_u16();
        let verification_tag = reader.get_u32();
        let their_checksum = reader.get_u32_le();

        let mut chunks = vec![];
        let mut offset = PACKET_HEADER_SIZE;
        loop {
            // Exact match, no more chunks
            if offset == raw.len() {
                break;
            } else if offset + CHUNK_HEADER_SIZE > raw.len() {
                return Err(Error::ErrParseSctpChunkNotEnoughData);
            }

            let ct = ChunkType(raw[offset]);
            let c: Box<dyn Chunk> = match ct {
                CT_INIT => Box::new(ChunkInit::unmarshal(&raw.slice(offset..))?),
                CT_INIT_ACK => Box::new(ChunkInit::unmarshal(&raw.slice(offset..))?),
                CT_ABORT => Box::new(ChunkAbort::unmarshal(&raw.slice(offset..))?),
                CT_COOKIE_ECHO => Box::new(ChunkCookieEcho::unmarshal(&raw.slice(offset..))?),
                CT_COOKIE_ACK => Box::new(ChunkCookieAck::unmarshal(&raw.slice(offset..))?),
                CT_HEARTBEAT => Box::new(ChunkHeartbeat::unmarshal(&raw.slice(offset..))?),
                CT_PAYLOAD_DATA => Box::new(ChunkPayloadData::unmarshal(&raw.slice(offset..))?),
                CT_SACK => Box::new(ChunkSelectiveAck::unmarshal(&raw.slice(offset..))?),
                CT_RECONFIG => Box::new(ChunkReconfig::unmarshal(&raw.slice(offset..))?),
                CT_FORWARD_TSN => Box::new(ChunkForwardTsn::unmarshal(&raw.slice(offset..))?),
                CT_ERROR => Box::new(ChunkError::unmarshal(&raw.slice(offset..))?),
                CT_SHUTDOWN => Box::new(ChunkShutdown::unmarshal(&raw.slice(offset..))?),
                CT_SHUTDOWN_ACK => Box::new(ChunkShutdownAck::unmarshal(&raw.slice(offset..))?),
                CT_SHUTDOWN_COMPLETE => {
                    Box::new(ChunkShutdownComplete::unmarshal(&raw.slice(offset..))?)
                }
                _ => return Err(Error::ErrUnmarshalUnknownChunkType),
            };

            let chunk_value_padding = get_padding_size(c.value_length());
            offset += CHUNK_HEADER_SIZE + c.value_length() + chunk_value_padding;
            chunks.push(c);
        }

        let our_checksum = generate_packet_checksum(raw);
        if their_checksum != our_checksum {
            return Err(Error::ErrChecksumMismatch);
        }

        Ok(Packet {
            source_port,
            destination_port,
            verification_tag,
            chunks,
        })
    }

    pub(crate) fn marshal_to(&self, writer: &mut BytesMut) -> Result<usize, Error> {
        // Populate static headers
        // 8-12 is Checksum which will be populated when packet is complete
        writer.put_u16(self.source_port);
        writer.put_u16(self.destination_port);
        writer.put_u32(self.verification_tag);

        // Populate chunks
        let mut raw = BytesMut::new();
        for c in &self.chunks {
            let chunk_raw = c.marshal()?;
            raw.extend(chunk_raw);

            let padding_needed = get_padding_size(raw.len());
            if padding_needed != 0 {
                raw.extend(vec![0u8; padding_needed]);
            }
        }
        let raw = raw.freeze();

        let mut hasher = crc32::Digest::new(crc32::CASTAGNOLI);
        hasher.write(&writer.to_vec());
        hasher.write(&FOUR_ZEROES);
        hasher.write(&raw[..]);
        let checksum = hasher.sum32();

        // Checksum is already in BigEndian
        // Using LittleEndian stops it from being flipped
        writer.put_u32_le(checksum);
        writer.extend(raw);

        Ok(writer.len())
    }

    pub(crate) fn marshal(&self) -> Result<Bytes, Error> {
        let mut buf = BytesMut::with_capacity(PACKET_HEADER_SIZE);
        self.marshal_to(&mut buf)?;
        Ok(buf.freeze())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_packet_unmarshal() -> Result<(), Error> {
        let result = Packet::unmarshal(&Bytes::new());
        assert!(
            result.is_err(),
            "Unmarshal should fail when a packet is too small to be SCTP"
        );

        let header_only = Bytes::from_static(&[
            0x13, 0x88, 0x13, 0x88, 0x00, 0x00, 0x00, 0x00, 0x06, 0xa9, 0x00, 0xe1,
        ]);
        let pkt = Packet::unmarshal(&header_only)?;
        //assert!(result.o(), "Unmarshal failed for SCTP packet with no chunks: {}", result);
        assert_eq!(
            pkt.source_port, 5000,
            "Unmarshal passed for SCTP packet, but got incorrect source port exp: {} act: {}",
            5000, pkt.source_port
        );
        assert_eq!(
            pkt.destination_port, 5000,
            "Unmarshal passed for SCTP packet, but got incorrect destination port exp: {} act: {}",
            5000, pkt.destination_port
        );
        assert_eq!(
            pkt.verification_tag, 0,
            "Unmarshal passed for SCTP packet, but got incorrect verification tag exp: {} act: {}",
            0, pkt.verification_tag
        );

        let raw_chunk = Bytes::from_static(&[
            0x13, 0x88, 0x13, 0x88, 0x00, 0x00, 0x00, 0x00, 0x81, 0x46, 0x9d, 0xfc, 0x01, 0x00,
            0x00, 0x56, 0x55, 0xb9, 0x64, 0xa5, 0x00, 0x02, 0x00, 0x00, 0x04, 0x00, 0x08, 0x00,
            0xe8, 0x6d, 0x10, 0x30, 0xc0, 0x00, 0x00, 0x04, 0x80, 0x08, 0x00, 0x09, 0xc0, 0x0f,
            0xc1, 0x80, 0x82, 0x00, 0x00, 0x00, 0x80, 0x02, 0x00, 0x24, 0x9f, 0xeb, 0xbb, 0x5c,
            0x50, 0xc9, 0xbf, 0x75, 0x9c, 0xb1, 0x2c, 0x57, 0x4f, 0xa4, 0x5a, 0x51, 0xba, 0x60,
            0x17, 0x78, 0x27, 0x94, 0x5c, 0x31, 0xe6, 0x5d, 0x5b, 0x09, 0x47, 0xe2, 0x22, 0x06,
            0x80, 0x04, 0x00, 0x06, 0x00, 0x01, 0x00, 0x00, 0x80, 0x03, 0x00, 0x06, 0x80, 0xc1,
            0x00, 0x00,
        ]);

        Packet::unmarshal(&raw_chunk)?;

        Ok(())
    }

    #[test]
    fn test_packet_marshal() -> Result<(), Error> {
        let header_only = Bytes::from_static(&[
            0x13, 0x88, 0x13, 0x88, 0x00, 0x00, 0x00, 0x00, 0x06, 0xa9, 0x00, 0xe1,
        ]);
        let pkt = Packet::unmarshal(&header_only)?;
        let header_only_marshaled = pkt.marshal()?;
        assert_eq!(header_only, header_only_marshaled, "Unmarshal/Marshaled header only packet did not match \nheaderOnly: {:?} \nheader_only_marshaled {:?}", header_only, header_only_marshaled);

        Ok(())
    }

    /*fn BenchmarkPacketGenerateChecksum(b *testing.B) {
        var data [1024]byte

        for i := 0; i < b.N; i++ {
            _ = generatePacketChecksum(data[:])
        }
    }*/
}
