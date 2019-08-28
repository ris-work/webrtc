#[macro_use]
extern crate lazy_static;

pub mod errors;
pub mod goodbye;
pub mod header;
pub mod packet;
pub mod rapid_resynchronization_request;
pub mod raw_packet;
pub mod receiver_report;
pub mod reception_report;
pub mod sender_report;
pub mod source_description;
pub mod transport_layer_nack;

// getPadding Returns the padding required to make the length a multiple of 4
fn get_padding(len: usize) -> usize {
    if len % 4 == 0 {
        0
    } else {
        4 - (len % 4)
    }
}
