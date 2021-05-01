use super::{chunk_header::*, chunk_type::*, *};
use crate::error_cause::*;

use bytes::{Bytes, BytesMut};
use std::fmt;

///Abort represents an SCTP Chunk of type ABORT
///
///The ABORT chunk is sent to the peer of an association to close the
///association.  The ABORT chunk may contain Cause Parameters to inform
///the receiver about the reason of the abort.  DATA chunks MUST NOT be
///bundled with ABORT.  Control chunks (except for INIT, INIT ACK, and
///SHUTDOWN COMPLETE) MAY be bundled with an ABORT, but they MUST be
///placed before the ABORT in the SCTP packet or they will be ignored by
///the receiver.
///
/// 0                   1                   2                   3
/// 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///|   Type = 6    |Reserved     |T|           Length              |
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
///|                                                               |
///|                   zero or more Error Causes                   |
///|                                                               |
///+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
#[derive(Debug, Clone)]
pub(crate) struct ChunkAbort {
    error_causes: Vec<ErrorCause>,
}

/// String makes chunkAbort printable
impl fmt::Display for ChunkAbort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut res = vec![self.header().to_string()];

        for cause in &self.error_causes {
            res.push(format!(" - {}", cause.to_string()));
        }

        write!(f, "{}", res.join("\n"))
    }
}

impl Chunk for ChunkAbort {
    fn unmarshal(raw: &Bytes) -> Result<Self, Error> {
        let header = ChunkHeader::unmarshal(raw)?;

        if header.typ != ChunkType::Abort {
            return Err(Error::ErrChunkTypeNotAbort);
        }

        let mut error_causes = vec![];
        let mut offset = CHUNK_HEADER_SIZE;
        while offset + 4 <= raw.len() {
            let e = ErrorCause::unmarshal(&raw.slice(offset..))?;
            offset += e.length();
            error_causes.push(e);
        }

        Ok(ChunkAbort { error_causes })
    }

    fn marshal_to(&self, buf: &mut BytesMut) -> Result<usize, Error> {
        self.header().marshal_to(buf)?;
        for ec in &self.error_causes {
            buf.extend(ec.marshal());
        }
        Ok(buf.len())
    }

    fn check(&self) -> Result<bool, Error> {
        Ok(false)
    }

    fn value_length(&self) -> usize {
        self.error_causes
            .iter()
            .fold(0, |length, ec| length + ec.length())
    }
}

impl ChunkAbort {
    pub(crate) fn header(&self) -> ChunkHeader {
        ChunkHeader {
            typ: ChunkType::Abort,
            flags: 0,
            value_length: self.value_length() as u16,
        }
    }
}
