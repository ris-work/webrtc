#[cfg(test)]
mod uattrs_test;

use crate::attributes::*;
use crate::errors::*;
use crate::message::*;

use util::Error;

use std::fmt;

// UnknownAttributes represents UNKNOWN-ATTRIBUTES attribute.
//
// RFC 5389 Section 15.9
pub struct UnknownAttributes(Vec<AttrType>);

impl fmt::Display for UnknownAttributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            write!(f, "<nil>")
        } else {
            let mut s = vec![];
            for t in &self.0 {
                s.push(t.to_string());
            }
            write!(f, "{}", s.join(", "))
        }
    }
}

// type size is 16 bit.
const ATTR_TYPE_SIZE: usize = 2;

impl UnknownAttributes {
    // add_to adds UNKNOWN-ATTRIBUTES attribute to message.
    pub fn add_to(&self, m: &mut Message) -> Result<(), Error> {
        let mut v = Vec::with_capacity(ATTR_TYPE_SIZE * 20); // 20 should be enough
                                                             // If len(a.Types) > 20, there will be allocations.
        for t in &self.0 {
            v.extend_from_slice(&t.value().to_be_bytes());
        }
        m.add(ATTR_UNKNOWN_ATTRIBUTES, &v);
        Ok(())
    }

    // GetFrom parses UNKNOWN-ATTRIBUTES from message.
    pub fn get_from(&mut self, m: &Message) -> Result<(), Error> {
        let v = m.get(ATTR_UNKNOWN_ATTRIBUTES)?;
        if v.len() % ATTR_TYPE_SIZE != 0 {
            return Err(ERR_BAD_UNKNOWN_ATTRS_SIZE.clone());
        }
        self.0.clear();
        let mut first = 0usize;
        while first < v.len() {
            let last = first + ATTR_TYPE_SIZE;
            self.0
                .push(AttrType(u16::from_be_bytes([v[first], v[first + 1]])));
            first = last;
        }
        Ok(())
    }
}