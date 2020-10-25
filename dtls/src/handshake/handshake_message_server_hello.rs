#[cfg(test)]
mod handshake_message_server_hello_test;

use super::handshake_random::*;
use super::*;
use crate::cipher_suite::*;
use crate::compression_methods::*;
use crate::extension::*;
use crate::record_layer::record_layer_header::*;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

use std::fmt;
use std::io::{BufReader, BufWriter};

/*
The server will send this message in response to a ClientHello
message when it was able to find an acceptable set of algorithms.
If it cannot find such a match, it will respond with a handshake
failure alert.
https://tools.ietf.org/html/rfc5246#section-7.4.1.3
*/
pub struct HandshakeMessageServerHello {
    version: ProtocolVersion,
    random: HandshakeRandom,

    cipher_suite: Box<dyn CipherSuite>,
    compression_method: CompressionMethodId,
    extensions: Vec<Extension>,
}

impl PartialEq for HandshakeMessageServerHello {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
            && self.random == other.random
            && self.compression_method == other.compression_method
            && self.extensions == other.extensions
            && self.cipher_suite.id() == other.cipher_suite.id()
    }
}

impl fmt::Debug for HandshakeMessageServerHello {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = vec![
            format!("version: {:?} random: {:?}", self.version, self.random),
            format!("cipher_suites: {:?}", self.cipher_suite.to_string()),
            format!("compression_method: {:?}", self.compression_method),
            format!("extensions: {:?}", self.extensions),
        ];
        write!(f, "{}", s.join(" "))
    }
}

impl HandshakeMessageServerHello {
    fn handshake_type() -> HandshakeType {
        HandshakeType::ServerHello
    }

    pub fn marshal<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        writer.write_u8(self.version.major)?;
        writer.write_u8(self.version.minor)?;
        self.random.marshal(writer)?;

        // SessionID
        writer.write_u8(0x00)?;

        writer.write_u16::<BigEndian>(self.cipher_suite.id() as u16)?;

        writer.write_u8(self.compression_method as u8)?;

        let mut extension_buffer = vec![];
        {
            let mut extension_writer = BufWriter::<&mut Vec<u8>>::new(extension_buffer.as_mut());
            for extension in &self.extensions {
                extension.marshal(&mut extension_writer)?;
            }
        }

        writer.write_u16::<BigEndian>(extension_buffer.len() as u16)?;
        writer.write_all(&extension_buffer)?;

        Ok(())
    }

    pub fn unmarshal<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let major = reader.read_u8()?;
        let minor = reader.read_u8()?;
        let random = HandshakeRandom::unmarshal(reader)?;

        // Session ID
        reader.read_u8()?;

        let id: CipherSuiteID = reader.read_u16::<BigEndian>()?.into();
        let cipher_suite = cipher_suite_for_id(id)?;

        let compression_method = reader.read_u8()?.into();
        let mut extensions = vec![];

        let extension_buffer_len = reader.read_u16::<BigEndian>()? as usize;
        let mut extension_buffer = vec![0u8; extension_buffer_len];
        reader.read_exact(&mut extension_buffer)?;

        let mut extension_reader = BufReader::new(extension_buffer.as_slice());
        let mut offset = 0;
        while offset < extension_buffer_len {
            let extension = Extension::unmarshal(&mut extension_reader)?;
            extensions.push(extension);

            let extension_len =
                u16::from_be_bytes([extension_buffer[offset + 2], extension_buffer[offset + 3]])
                    as usize;
            offset += 4 + extension_len;
        }

        Ok(HandshakeMessageServerHello {
            version: ProtocolVersion { major, minor },
            random,

            cipher_suite,
            compression_method,
            extensions,
        })
    }
}
