#[cfg(test)]
mod handshake_message_certificate_request_test;

use super::*;
use crate::client_certificate_type::*;
use crate::signature_hash_algorithm::*;

use std::io::{Read, Write};

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};

use util::Error;

/*
A non-anonymous server can optionally request a certificate from
the client, if appropriate for the selected cipher suite.  This
message, if sent, will immediately follow the ServerKeyExchange
message (if it is sent; otherwise, this message follows the
server's Certificate message).
*/
#[derive(Clone, Debug, PartialEq)]
pub struct HandshakeMessageCertificateRequest {
    certificate_types: Vec<ClientCertificateType>,
    signature_hash_algorithms: Vec<SignatureHashAlgorithm>,
}

const HANDSHAKE_MESSAGE_CERTIFICATE_REQUEST_MIN_LENGTH: usize = 5;

impl HandshakeMessageCertificateRequest {
    fn handshake_type() -> HandshakeType {
        HandshakeType::CertificateRequest
    }

    pub fn marshal<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        writer.write_u8(self.certificate_types.len() as u8)?;
        for v in &self.certificate_types {
            writer.write_u8(*v as u8)?;
        }

        writer.write_u16::<BigEndian>(2 * self.signature_hash_algorithms.len() as u16)?;
        for v in &self.signature_hash_algorithms {
            writer.write_u8(v.hash as u8)?;
            writer.write_u8(v.signature as u8)?;
        }

        writer.write_all(&[0x00, 0x00])?; // Distinguished Names Length

        Ok(())
    }

    pub fn unmarshal<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let certificate_types_length = reader.read_u8()?;

        let mut certificate_types = vec![];
        for _ in 0..certificate_types_length {
            let cert_type = reader.read_u8()?.into();
            certificate_types.push(cert_type);
        }

        let signature_hash_algorithms_length = reader.read_u16::<BigEndian>()?;

        let mut signature_hash_algorithms = vec![];
        for _ in (0..signature_hash_algorithms_length).step_by(2) {
            let hash = reader.read_u8()?.into();
            let signature = reader.read_u8()?.into();

            signature_hash_algorithms.push(SignatureHashAlgorithm { hash, signature });
        }

        Ok(HandshakeMessageCertificateRequest {
            certificate_types,
            signature_hash_algorithms,
        })
    }
}
