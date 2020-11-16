#[cfg(test)]
mod handshake_message_finished_test;

use super::*;

use std::io::{Read, Write};

use util::Error;

#[derive(Clone, Debug, PartialEq)]
pub struct HandshakeMessageFinished {
    pub(crate) verify_data: Vec<u8>,
}

impl HandshakeMessageFinished {
    fn handshake_type() -> HandshakeType {
        HandshakeType::Finished
    }

    pub fn marshal<W: Write>(&self, writer: &mut W) -> Result<(), Error> {
        writer.write_all(&self.verify_data)?;

        Ok(())
    }

    pub fn unmarshal<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let mut verify_data: Vec<u8> = vec![];
        reader.read_to_end(&mut verify_data)?;

        Ok(HandshakeMessageFinished { verify_data })
    }
}
