pub mod handshake_cache;
pub mod handshake_header;
pub mod handshake_message_certificate;
pub mod handshake_message_certificate_request;
pub mod handshake_message_certificate_verify;
pub mod handshake_message_client_hello;
pub mod handshake_message_client_key_exchange;
pub mod handshake_message_finished;
pub mod handshake_message_hello_verify_request;
pub mod handshake_message_server_hello;
pub mod handshake_message_server_hello_done;
pub mod handshake_message_server_key_exchange;
pub mod handshake_random;

#[cfg(test)]
mod handshake_test;

use std::fmt;
use std::io::{Read, Write};

use util::Error;

use super::content::*;
use super::errors::*;

use handshake_header::*;
use handshake_message_certificate::*;
use handshake_message_certificate_request::*;
use handshake_message_certificate_verify::*;
use handshake_message_client_hello::*;
use handshake_message_client_key_exchange::*;
use handshake_message_finished::*;
use handshake_message_hello_verify_request::*;
use handshake_message_server_hello::*;
use handshake_message_server_hello_done::*;
use handshake_message_server_key_exchange::*;

// https://tools.ietf.org/html/rfc5246#section-7.4
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum HandshakeType {
    HelloRequest = 0,
    ClientHello = 1,
    ServerHello = 2,
    HelloVerifyRequest = 3,
    Certificate = 11,
    ServerKeyExchange = 12,
    CertificateRequest = 13,
    ServerHelloDone = 14,
    CertificateVerify = 15,
    ClientKeyExchange = 16,
    Finished = 20,
    Invalid,
}

impl fmt::Display for HandshakeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            HandshakeType::HelloRequest => write!(f, "HelloRequest"),
            HandshakeType::ClientHello => write!(f, "ClientHello"),
            HandshakeType::ServerHello => write!(f, "ServerHello"),
            HandshakeType::HelloVerifyRequest => write!(f, "HelloVerifyRequest"),
            HandshakeType::Certificate => write!(f, "TypeCertificate"),
            HandshakeType::ServerKeyExchange => write!(f, "ServerKeyExchange"),
            HandshakeType::CertificateRequest => write!(f, "CertificateRequest"),
            HandshakeType::ServerHelloDone => write!(f, "ServerHelloDone"),
            HandshakeType::CertificateVerify => write!(f, "CertificateVerify"),
            HandshakeType::ClientKeyExchange => write!(f, "ClientKeyExchange"),
            HandshakeType::Finished => write!(f, "Finished"),
            HandshakeType::Invalid => write!(f, "Invalid"),
        }
    }
}

impl From<u8> for HandshakeType {
    fn from(val: u8) -> Self {
        match val {
            0 => HandshakeType::HelloRequest,
            1 => HandshakeType::ClientHello,
            2 => HandshakeType::ServerHello,
            3 => HandshakeType::HelloVerifyRequest,
            11 => HandshakeType::Certificate,
            12 => HandshakeType::ServerKeyExchange,
            13 => HandshakeType::CertificateRequest,
            14 => HandshakeType::ServerHelloDone,
            15 => HandshakeType::CertificateVerify,
            16 => HandshakeType::ClientKeyExchange,
            20 => HandshakeType::Finished,
            _ => HandshakeType::Invalid,
        }
    }
}

#[derive(PartialEq, Debug)]
pub enum HandshakeMessage {
    //HelloRequest(errNotImplemented),
    ClientHello(HandshakeMessageClientHello),
    ServerHello(HandshakeMessageServerHello),
    HelloVerifyRequest(HandshakeMessageHelloVerifyRequest),
    Certificate(HandshakeMessageCertificate),
    ServerKeyExchange(HandshakeMessageServerKeyExchange),
    CertificateRequest(HandshakeMessageCertificateRequest),
    ServerHelloDone(HandshakeMessageServerHelloDone),
    CertificateVerify(HandshakeMessageCertificateVerify),
    ClientKeyExchange(HandshakeMessageClientKeyExchange),
    Finished(HandshakeMessageFinished),
}

impl HandshakeMessage {
    pub fn marshal<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        match self {
            HandshakeMessage::ClientHello(msg) => msg.marshal(writer)?,
            HandshakeMessage::ServerHello(msg) => msg.marshal(writer)?,
            HandshakeMessage::HelloVerifyRequest(msg) => msg.marshal(writer)?,
            HandshakeMessage::Certificate(msg) => msg.marshal(writer)?,
            HandshakeMessage::ServerKeyExchange(msg) => msg.marshal(writer)?,
            HandshakeMessage::CertificateRequest(msg) => msg.marshal(writer)?,
            HandshakeMessage::ServerHelloDone(msg) => msg.marshal(writer)?,
            HandshakeMessage::CertificateVerify(msg) => msg.marshal(writer)?,
            HandshakeMessage::ClientKeyExchange(msg) => msg.marshal(writer)?,
            HandshakeMessage::Finished(msg) => msg.marshal(writer)?,
        }

        Ok(())
    }
}

// The handshake protocol is responsible for selecting a cipher spec and
// generating a master secret, which together comprise the primary
// cryptographic parameters associated with a secure session.  The
// handshake protocol can also optionally authenticate parties who have
// certificates signed by a trusted certificate authority.
// https://tools.ietf.org/html/rfc5246#section-7.3
#[derive(PartialEq, Debug)]
pub struct Handshake {
    handshake_header: HandshakeHeader,
    handshake_message: HandshakeMessage,
}

impl Handshake {
    pub fn content_type(&self) -> ContentType {
        ContentType::Handshake
    }

    pub fn marshal<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.handshake_header.marshal(writer)?;
        self.handshake_message.marshal(writer)?;
        Ok(())
    }

    pub fn unmarshal<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let handshake_header = HandshakeHeader::unmarshal(reader)?;

        let handshake_message = match handshake_header.handshake_type {
            HandshakeType::ClientHello => {
                HandshakeMessage::ClientHello(HandshakeMessageClientHello::unmarshal(reader)?)
            }
            HandshakeType::ServerHello => {
                HandshakeMessage::ServerHello(HandshakeMessageServerHello::unmarshal(reader)?)
            }
            HandshakeType::HelloVerifyRequest => HandshakeMessage::HelloVerifyRequest(
                HandshakeMessageHelloVerifyRequest::unmarshal(reader)?,
            ),
            HandshakeType::Certificate => {
                HandshakeMessage::Certificate(HandshakeMessageCertificate::unmarshal(reader)?)
            }
            HandshakeType::ServerKeyExchange => HandshakeMessage::ServerKeyExchange(
                HandshakeMessageServerKeyExchange::unmarshal(reader)?,
            ),
            HandshakeType::CertificateRequest => HandshakeMessage::CertificateRequest(
                HandshakeMessageCertificateRequest::unmarshal(reader)?,
            ),
            HandshakeType::ServerHelloDone => HandshakeMessage::ServerHelloDone(
                HandshakeMessageServerHelloDone::unmarshal(reader)?,
            ),
            HandshakeType::CertificateVerify => HandshakeMessage::CertificateVerify(
                HandshakeMessageCertificateVerify::unmarshal(reader)?,
            ),
            HandshakeType::ClientKeyExchange => HandshakeMessage::ClientKeyExchange(
                HandshakeMessageClientKeyExchange::unmarshal(reader)?,
            ),
            HandshakeType::Finished => {
                HandshakeMessage::Finished(HandshakeMessageFinished::unmarshal(reader)?)
            }
            _ => return Err(ERR_NOT_IMPLEMENTED.clone()),
        };

        Ok(Handshake {
            handshake_header,
            handshake_message,
        })
    }
}
