use super::header::*;
use super::question::*;
use super::resource::*;
use super::*;
use crate::errors::*;

use util::Error;

use std::collections::HashMap;

// A Builder allows incrementally packing a DNS message.
//
// Example usage:
//	b := NewBuilder(Header{...})
//	b.enable_compression()
//	// Optionally start a section and add things to that section.
//	// Repeat adding sections as necessary.
//	buf, err := b.Finish()
//	// If err is nil, buf[2:] will contain the built bytes.
#[derive(Default)]
pub struct Builder {
    // msg is the storage for the message being built.
    pub msg: Option<Vec<u8>>,

    // section keeps track of the current section being built.
    pub section: Section,

    // header keeps track of what should go in the header when Finish is
    // called.
    pub header: HeaderInternal,

    // start is the starting index of the bytes allocated in msg for header.
    pub start: usize,

    // compression is a mapping from name suffixes to their starting index
    // in msg.
    pub compression: Option<HashMap<String, usize>>,
}

impl Builder {
    // NewBuilder creates a new builder with compression disabled.
    //
    // Note: Most users will want to immediately enable compression with the
    // enable_compression method. See that method's comment for why you may or may
    // not want to enable compression.
    //
    // The DNS message is appended to the provided initial buffer buf (which may be
    // nil) as it is built. The final message is returned by the (*Builder).Finish
    // method, which may return the same underlying array if there was sufficient
    // capacity in the slice.
    pub fn new(h: &Header) -> Self {
        let (id, bits) = h.pack();

        Builder {
            msg: Some(vec![0; HEADER_LEN]),
            start: 0,
            section: Section::Header,
            header: HeaderInternal {
                id,
                bits,
                ..Default::default()
            },
            compression: None,
        }

        //var hb [HEADER_LEN]byte
        //b.msg = append(b.msg, hb[:]...)
        //return b
    }

    // enable_compression enables compression in the Builder.
    //
    // Leaving compression disabled avoids compression related allocations, but can
    // result in larger message sizes. Be careful with this mode as it can cause
    // messages to exceed the UDP size limit.
    //
    // According to RFC 1035, section 4.1.4, the use of compression is optional, but
    // all implementations must accept both compressed and uncompressed DNS
    // messages.
    //
    // Compression should be enabled before any sections are added for best results.
    pub fn enable_compression(&mut self) {
        self.compression = Some(HashMap::new());
    }

    fn start_check(&self, section: Section) -> Result<(), Error> {
        if self.section <= Section::NotStarted {
            return Err(ERR_NOT_STARTED.to_owned());
        }
        if self.section > section {
            return Err(ERR_SECTION_DONE.to_owned());
        }

        Ok(())
    }

    // start_questions prepares the builder for packing Questions.
    pub fn start_questions(&mut self) -> Result<(), Error> {
        self.start_check(Section::Questions)?;
        self.section = Section::Questions;
        Ok(())
    }

    // start_answers prepares the builder for packing Answers.
    pub fn start_answers(&mut self) -> Result<(), Error> {
        self.start_check(Section::Answers)?;
        self.section = Section::Answers;
        Ok(())
    }

    // start_authorities prepares the builder for packing Authorities.
    pub fn start_authorities(&mut self) -> Result<(), Error> {
        self.start_check(Section::Authorities)?;
        self.section = Section::Authorities;
        Ok(())
    }

    // start_additionals prepares the builder for packing Additionals.
    pub fn start_additionals(&mut self) -> Result<(), Error> {
        self.start_check(Section::Additionals)?;
        self.section = Section::Additionals;
        Ok(())
    }

    fn increment_section_count(&mut self) -> Result<(), Error> {
        let section = self.section;
        let (count, err) = match section {
            Section::Questions => (
                &mut self.header.questions,
                ERR_TOO_MANY_QUESTIONS.to_owned(),
            ),
            Section::Answers => (&mut self.header.answers, ERR_TOO_MANY_ANSWERS.to_owned()),
            Section::Authorities => (
                &mut self.header.authorities,
                ERR_TOO_MANY_AUTHORITIES.to_owned(),
            ),
            Section::Additionals => (
                &mut self.header.additionals,
                ERR_TOO_MANY_ADDITIONALS.to_owned(),
            ),
            Section::NotStarted => return Err(ERR_NOT_STARTED.to_owned()),
            Section::Done => return Err(ERR_SECTION_DONE.to_owned()),
            Section::Header => return Err(ERR_SECTION_HEADER.to_owned()),
        };

        if *count == u16::MAX {
            Err(err)
        } else {
            *count += 1;
            Ok(())
        }
    }

    // question adds a single question.
    pub fn add_question(&mut self, q: &Question) -> Result<(), Error> {
        if self.section < Section::Questions {
            return Err(ERR_NOT_STARTED.to_owned());
        }
        if self.section > Section::Questions {
            return Err(ERR_SECTION_DONE.to_owned());
        }
        let msg = self.msg.take();
        if let Some(mut msg) = msg {
            msg = q.pack(msg, &mut self.compression, self.start)?;
            self.increment_section_count()?;
            self.msg = Some(msg);
        }

        Ok(())
    }

    fn check_resource_section(&self) -> Result<(), Error> {
        if self.section < Section::Answers {
            return Err(ERR_NOT_STARTED.to_owned());
        }
        if self.section > Section::Additionals {
            return Err(ERR_SECTION_DONE.to_owned());
        }
        Ok(())
    }

    // Resource adds a single resource.
    pub fn add_resource(&mut self, r: &mut Resource) -> Result<(), Error> {
        self.check_resource_section()?;

        r.header.typ = r.body.real_type();
        let buf = r.body.pack(vec![], &mut self.compression, self.start)?;
        r.header.length = buf.len() as u16;

        let msg = self.msg.take();
        if let Some(mut msg) = msg {
            msg = r.header.pack(msg, &mut self.compression, self.start)?;
            self.increment_section_count()?;
            msg.extend_from_slice(&buf);
            self.msg = Some(msg);
        }
        Ok(())
    }

    // Finish ends message building and generates a binary message.
    pub fn finish(&mut self) -> Result<Vec<u8>, Error> {
        if self.section < Section::Header {
            return Err(ERR_NOT_STARTED.to_owned());
        }
        self.section = Section::Done;

        // Space for the header was allocated in NewBuilder.
        let buf = self.header.pack(vec![])?;
        assert_eq!(buf.len(), HEADER_LEN);
        if let Some(mut msg) = self.msg.take() {
            msg[..HEADER_LEN].copy_from_slice(&buf[..HEADER_LEN]);
            Ok(msg)
        } else {
            Err(ERR_EMPTY_BUILDER_MSG.to_owned())
        }
    }
}
