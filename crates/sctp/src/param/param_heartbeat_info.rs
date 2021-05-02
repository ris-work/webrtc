use super::{param_header::*, param_type::*, *};

use bytes::{Bytes, BytesMut};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParamHeartbeatInfo {
    pub(crate) heartbeat_information: Bytes,
}

impl fmt::Display for ParamHeartbeatInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {:?}", self.header(), self.heartbeat_information)
    }
}

impl Param for ParamHeartbeatInfo {
    fn header(&self) -> ParamHeader {
        ParamHeader {
            typ: ParamType::HeartbeatInfo,
            value_length: self.value_length() as u16,
        }
    }

    fn unmarshal(raw: &Bytes) -> Result<Self, Error> {
        let _ = ParamHeader::unmarshal(raw)?;
        let heartbeat_information = raw.slice(PARAM_HEADER_LENGTH..);
        Ok(ParamHeartbeatInfo {
            heartbeat_information,
        })
    }

    fn marshal_to(&self, buf: &mut BytesMut) -> Result<usize, Error> {
        self.header().marshal_to(buf)?;
        buf.extend(self.heartbeat_information.clone());
        Ok(buf.len())
    }

    fn value_length(&self) -> usize {
        self.heartbeat_information.len()
    }
}
