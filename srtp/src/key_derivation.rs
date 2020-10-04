#[cfg(test)]
mod key_derivation_test;

use aes::block_cipher_trait::generic_array::GenericArray;
use aes::block_cipher_trait::BlockCipher;
use aes::Aes128;

use std::io::BufWriter;

use byteorder::{BigEndian, WriteBytesExt};

use util::Error;

pub(crate) fn aes_cm_key_derivation(
    label: u8,
    master_key: &[u8],
    master_salt: &[u8],
    index_over_kdr: usize,
    out_len: usize,
) -> Result<Vec<u8>, Error> {
    if index_over_kdr != 0 {
        // 24-bit "index DIV kdr" must be xored to prf input.
        return Err(Error::new(
            "index_over_kdr > 0 is not supported yet".to_owned(),
        ));
    }

    // https://tools.ietf.org/html/rfc3711#appendix-B.3
    // The input block for AES-CM is generated by exclusive-oring the master salt with the
    // concatenation of the encryption key label 0x00 with (index DIV kdr),
    // - index is 'rollover count' and DIV is 'divided by'

    let n_master_key = master_key.len();
    let n_master_salt = master_salt.len();

    let mut prf_in = vec![0u8; n_master_key];
    prf_in[..n_master_salt].copy_from_slice(master_salt);

    prf_in[7] ^= label;

    //The resulting value is then AES encrypted using the master key to get the cipher key.
    let key = GenericArray::from_slice(master_key);
    let block = Aes128::new(&key);

    let mut out = vec![0u8; ((out_len + n_master_key) / n_master_key) * n_master_key];
    let mut i = 0u16;
    for n in (0..out_len).step_by(n_master_key) {
        //BigEndian.PutUint16(prfIn[nMasterKey-2:], i)
        prf_in[n_master_key - 2] = ((i >> 8) & 0xFF) as u8;
        prf_in[n_master_key - 1] = (i & 0xFF) as u8;

        out[n..n + n_master_key].copy_from_slice(&prf_in);
        let out_key = GenericArray::from_mut_slice(&mut out[n..n + n_master_key]);
        block.encrypt_block(out_key);

        i += 1;
    }

    Ok(out[..out_len].to_vec())
}

// Generate IV https://tools.ietf.org/html/rfc3711#section-4.1.1
// where the 128-bit integer value IV SHALL be defined by the SSRC, the
// SRTP packet index i, and the SRTP session salting key k_s, as below.
// - ROC = a 32-bit unsigned rollover counter (ROC), which records how many
// -       times the 16-bit RTP sequence number has been reset to zero after
// -       passing through 65,535
// i = 2^16 * ROC + SEQ
// IV = (salt*2 ^ 16) | (ssrc*2 ^ 64) | (i*2 ^ 16)
pub(crate) fn generate_counter(
    sequence_number: u16,
    rollover_counter: u32,
    ssrc: u32,
    session_salt: &[u8],
) -> Result<Vec<u8>, Error> {
    assert!(session_salt.len() <= 16);

    let mut counter: Vec<u8> = vec![0; 16];
    {
        let mut writer = BufWriter::<&mut [u8]>::new(counter[4..].as_mut());
        writer.write_u32::<BigEndian>(ssrc)?;
        writer.write_u32::<BigEndian>(rollover_counter)?;
        writer.write_u32::<BigEndian>((sequence_number as u32) << 16)?;
    }

    for i in 0..session_salt.len() {
        counter[i] ^= session_salt[i];
    }

    Ok(counter)
}
