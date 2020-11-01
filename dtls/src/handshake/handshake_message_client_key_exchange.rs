#[cfg(test)]
mod handshake_message_client_key_exchange_test;

use super::*;

use std::io::{Read, Write};

use byteorder::{BigEndian, WriteBytesExt};

use util::Error;

#[derive(Clone, Debug, PartialEq)]
pub struct HandshakeMessageClientKeyExchange {
    identity_hint: Vec<u8>,
    public_key: Vec<u8>,
}

impl HandshakeMessageClientKeyExchange {
    fn handshake_type() -> HandshakeType {
        HandshakeType::ClientKeyExchange
    }

    pub fn marshal<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        if (!self.identity_hint.is_empty() && !self.public_key.is_empty())
            || (self.identity_hint.is_empty() && self.public_key.is_empty())
        {
            return Err(ERR_INVALID_CLIENT_KEY_EXCHANGE.clone());
        }

        if !self.public_key.is_empty() {
            writer.write_u8(self.public_key.len() as u8)?;
            writer.write_all(&self.public_key)?;
        } else {
            writer.write_u16::<BigEndian>(self.identity_hint.len() as u16)?;
            writer.write_all(&self.identity_hint)?;
        }

        Ok(())
    }

    pub fn unmarshal<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let mut data = vec![];
        reader.read_to_end(&mut data)?;

        // If parsed as PSK return early and only populate PSK Identity Hint
        let psk_length = ((data[0] as u16) << 8) | data[1] as u16;
        if data.len() == psk_length as usize + 2 {
            return Ok(HandshakeMessageClientKeyExchange {
                identity_hint: data[2..].to_vec(),
                public_key: vec![],
            });
        }

        let public_key_length = data[0] as usize;
        if data.len() != public_key_length + 1 {
            return Err(ERR_BUFFER_TOO_SMALL.clone());
        }

        Ok(HandshakeMessageClientKeyExchange {
            identity_hint: vec![],
            public_key: data[1..].to_vec(),
        })
    }
}
